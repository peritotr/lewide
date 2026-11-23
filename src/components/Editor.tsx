// The query editor: CodeMirror 6 themed from the --syn-* tokens (see index.css),
// with a gutter bar marking the statement that Run would execute. It speaks
// SQL in each dialect, Cypher, and Elasticsearch's console (see languages.ts).
import { autocompletion, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { MSSQL, MySQL, PostgreSQL, sql, SQLite } from "@codemirror/lang-sql";
import { HighlightStyle, indentUnit, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { Compartment, EditorState, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  drawSelection,
  EditorView,
  gutter,
  GutterMarker,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  MatchDecorator,
  placeholder,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { useEffect, useRef, type RefObject } from "react";
import { msg, t } from "../i18n";
import { usePrefs } from "../prefs";
import { cypherLanguage, consoleLanguage } from "../lib/editor-modes";
import { splitFor, statementAtFor, type EditorLanguage } from "../lib/languages";
import type { SqlDialect, Statement } from "../lib/sql";

/** Completion kinds as lang-sql names them: column, table, function, keyword. */
const KIND: Record<string, string> = { property: "C", type: "T", function: "ƒ", keyword: "K", namespace: "S" };

export interface EditorHandle {
  /** The statement under the caret, which is what Run executes. */
  statementAtCaret(): Statement | null;
  /** The statements in the selection, or null when nothing is selected. */
  selectedStatements(): Statement[] | null;
  text(): string;
  replaceText(text: string): void;
  /** Moves the caret to a 0-based offset, or to the start of a 1-based line. */
  goTo(place: { offset?: number | null; line: number }): void;
  /** Moves the caret to a 1-based line and tints the line for a moment (AI05). */
  flashLine(line: number): void;
}

/** A finding of an AI review on one line (AI05): hollow diamond, dotted underline. */
export interface AiMark {
  /** 1-based line in the editor. */
  line: number;
  severity: "error" | "warning" | "note";
  /** The text to underline on that line, when the finding names one. */
  token: string | null;
  message: string;
}

/** Where the last run failed, to be marked in the text. */
export interface ErrorMark {
  /** 1-based line. */
  line: number;
  /** 0-based offset of the offending token, when the server gave one. */
  offset: number | null;
  message: string;
}

const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.operatorKeyword, tags.typeName], color: "var(--syn-kw)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--syn-str)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--syn-num)" },
  { tag: [tags.standard(tags.name), tags.function(tags.variableName)], color: "var(--syn-fn)" },
  { tag: [tags.lineComment, tags.blockComment], color: "var(--syn-com)", fontStyle: "italic" },
]);

const DIALECTS: Record<SqlDialect, typeof PostgreSQL> = { postgres: PostgreSQL, mysql: MySQL, sqlite: SQLite, sqlserver: MSSQL, clickhouse: MySQL, duckdb: PostgreSQL };

// The console's is an example request, not a sentence, so it is not translated.
const PLACEHOLDER: Record<EditorLanguage["kind"], () => string> = {
  sql: () => t(msg("Write SQL here")),
  cypher: () => t(msg("Write Cypher here")),
  console: () => "GET /index/_search",
};

// The SQL grammar does not mark function calls, so a name directly followed by
// "(" is coloured here, unless the grammar says it is a keyword (IN, VALUES, ...),
// a string or a comment.
const functionMark = Decoration.mark({ class: "cm-sql-function" });
const functionCalls = new MatchDecorator({
  regexp: /[A-Za-z_][A-Za-z0-9_]*(?=\()/g,
  decorate(add, from, to, _match, view) {
    const node = syntaxTree(view.state).resolveInner(from, 1).name;
    if (node === "Identifier" || node === "Type" || node === "Builtin") add(from, to, functionMark);
  },
});
const functionHighlight = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = functionCalls.createDeco(view);
    }
    update(update: ViewUpdate) {
      this.decorations = functionCalls.updateDeco(update, this.decorations);
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

class StatementMark extends GutterMarker {
  toDOM() {
    const mark = document.createElement("div");
    mark.className = "cm-stmt-mark";
    return mark;
  }
}
const statementMark = new StatementMark();

class ErrorDiamond extends GutterMarker {
  toDOM() {
    const mark = document.createElement("div");
    mark.className = "cm-error-diamond";
    return mark;
  }
}
const errorDiamond = new ErrorDiamond();

class AiDiamond extends GutterMarker {
  constructor(readonly severity: AiMark["severity"]) {
    super();
  }
  eq(other: AiDiamond) {
    return other.severity === this.severity;
  }
  toDOM() {
    const mark = document.createElement("div");
    mark.className = `cm-ai-diamond cm-ai-${this.severity}`;
    return mark;
  }
}
const aiDiamonds = { error: new AiDiamond("error"), warning: new AiDiamond("warning"), note: new AiDiamond("note") };

// The findings of the last review. Like the run error, any edit clears them.
const setAiMarks = StateEffect.define<AiMark[] | null>();
const aiField = StateField.define<AiMark[] | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setAiMarks)) return effect.value;
    return tr.docChanged ? null : value;
  },
});
const aiDecorations = EditorView.decorations.compute([aiField], (state) => {
  const marks = state.field(aiField);
  if (!marks) return Decoration.none;
  const ranges = [];
  for (const mark of marks) {
    if (!mark.token || mark.line < 1 || mark.line > state.doc.lines) continue;
    const line = state.doc.line(mark.line);
    const at = line.text.toLowerCase().indexOf(mark.token.toLowerCase());
    if (at < 0) continue;
    ranges.push(Decoration.mark({ class: `cm-ai-token cm-ai-${mark.severity}`, attributes: { title: `${t("AI review:")} ${mark.message}` } }).range(line.from + at, line.from + at + mark.token.length));
  }
  return Decoration.set(ranges, true);
});

// A line tinted for a moment after its finding is clicked.
const setFlash = StateEffect.define<number | null>();
const flashField = StateField.define<number | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setFlash)) return effect.value;
    return value;
  },
});
const flashDecorations = EditorView.decorations.compute([flashField], (state) => {
  const line = state.field(flashField);
  if (line === null || line < 1 || line > state.doc.lines) return Decoration.none;
  return Decoration.set([Decoration.line({ class: "cm-flash-line" }).range(state.doc.line(line).from)]);
});

// The error of the last run. Any edit clears it: the positions no longer hold.
const setError = StateEffect.define<ErrorMark | null>();
const errorField = StateField.define<ErrorMark | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setError)) return effect.value;
    return tr.docChanged ? null : value;
  },
});
const errorDecorations = EditorView.decorations.compute([errorField], (state) => {
  const error = state.field(errorField);
  if (!error || error.line < 1 || error.line > state.doc.lines) return Decoration.none;
  const line = state.doc.line(error.line);
  const ranges = [Decoration.line({ class: "cm-error-line" }).range(line.from)];
  if (error.offset !== null && error.offset >= line.from && error.offset < line.to) {
    // The offending word: from the reported position to the end of the identifier or token.
    const rest = state.sliceDoc(error.offset, line.to);
    const length = (/^[A-Za-z0-9_.$"`]+/.exec(rest)?.[0].length ?? 0) || 1;
    ranges.push(Decoration.mark({ class: "cm-error-token", attributes: { title: error.message } }).range(error.offset, error.offset + length));
  }
  return Decoration.set(ranges);
});

interface Props {
  language: EditorLanguage;
  /** Tables and their columns, offered as completions. Tables without loaded columns map to []. */
  schema: Record<string, string[]>;
  initialText: string;
  error: ErrorMark | null;
  /** Findings of the last AI review, while its panel is open. */
  aiMarks?: AiMark[] | null;
  handle: RefObject<EditorHandle | null>;
  onChange(text: string): void;
  onCaret(line: number, col: number): void;
  onRun(): void;
  onRunSelection(): void;
  onExplain(): void;
}

/** Mount with a `key` per tab: the editor keeps its own text after the first render. */
export function Editor(props: Props) {
  const host = useRef<HTMLDivElement>(null);
  // The latest callbacks, so the editor does not have to be rebuilt when they change.
  const latest = useRef(props);
  latest.current = props;
  const view = useRef<EditorView | null>(null);
  // The language sits in a compartment so the completion schema can change without rebuilding the editor.
  const language = useRef(new Compartment());
  // Only SQL gets completion from the schema and the function-call colour.
  const languageFor = (lang: EditorLanguage, schema: Record<string, string[]>) =>
    lang.kind === "sql" ? [sql({ dialect: DIALECTS[lang.dialect], schema, upperCaseKeywords: true }), functionHighlight] : lang.kind === "cypher" ? cypherLanguage : consoleLanguage;

  useEffect(() => {
    view.current?.dispatch({ effects: language.current.reconfigure(languageFor(props.language, props.schema)) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.schema]);

  useEffect(() => {
    view.current?.dispatch({ effects: setError.of(props.error) });
  }, [props.error]);

  useEffect(() => {
    view.current?.dispatch({ effects: setAiMarks.of(props.aiMarks ?? null) });
  }, [props.aiMarks]);

  useEffect(() => {
    const { language: lang, initialText, handle } = latest.current;
    // Settings › Editor, read when the editor is made.
    const prefs = usePrefs.getState();
    document.documentElement.style.setProperty("--completion-hint", JSON.stringify(t("↑↓ Move · ↵ Tab Accept · Esc Close")));
    const range = (state: EditorState) => statementAtFor(state.doc.toString(), state.selection.main.head, lang);
    const statement = StateField.define({
      create: range,
      update: (value, tr) => (tr.docChanged || tr.selection ? range(tr.state) : value),
    });
    const command = (run: () => void) => () => {
      run();
      return true;
    };

    const editor = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: initialText,
        extensions: [
          errorField,
          errorDecorations,
          aiField,
          aiDecorations,
          flashField,
          flashDecorations,
          gutter({
            class: "cm-error-gutter",
            lineMarker(view, line) {
              const number = view.state.doc.lineAt(line.from).number;
              if (view.state.field(errorField)?.line === number) return errorDiamond;
              // The most severe finding of the line, when it has several.
              const found = view.state.field(aiField)?.filter((m) => m.line === number) ?? [];
              const worst = found.find((m) => m.severity === "error") ?? found.find((m) => m.severity === "warning") ?? found[0];
              return worst ? aiDiamonds[worst.severity] : null;
            },
            lineMarkerChange: (update) => update.startState.field(errorField) !== update.state.field(errorField) || update.startState.field(aiField) !== update.state.field(aiField),
          }),
          lineNumbers(),
          statement,
          gutter({
            class: "cm-stmt-gutter",
            lineMarker(view, line) {
              if (!prefs.highlightStatement) return null;
              const current = view.state.field(statement);
              return current && line.from < current.to && line.to >= current.from ? statementMark : null;
            },
            lineMarkerChange: (update) => update.startState.field(statement) !== update.state.field(statement),
          }),
          EditorState.tabSize.of(prefs.tabWidth),
          indentUnit.of(" ".repeat(prefs.tabWidth)),
          ...(prefs.wordWrap ? [EditorView.lineWrapping] : []),
          history(),
          drawSelection(),
          highlightActiveLine(),
          highlightActiveLineGutter(),
          placeholder(PLACEHOLDER[lang.kind]()),
          language.current.of(languageFor(lang, latest.current.schema)),
          ...(prefs.autocomplete
            ? [
                autocompletion({
                  icons: false,
                  // The kind of each suggestion, as a letter in a box.
                  addToOptions: [
                    {
                      position: 20,
                      render(completion) {
                        const chip = document.createElement("span");
                        chip.className = "cm-completion-kind";
                        chip.textContent = KIND[completion.type ?? ""] ?? "K";
                        return chip;
                      },
                    },
                  ],
                }),
              ]
            : []),
          syntaxHighlighting(highlight),
          keymap.of([
            { key: "Shift-Mod-Enter", run: command(() => latest.current.onRunSelection()) },
            { key: "Mod-Enter", run: command(() => latest.current.onRun()) },
            { key: "Mod-e", run: command(() => latest.current.onExplain()) },
            ...completionKeymap,
            indentWithTab,
            ...defaultKeymap,
            ...historyKeymap,
          ]),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) latest.current.onChange(update.state.doc.toString());
            if (update.docChanged || update.selectionSet) {
              const head = update.state.selection.main.head;
              const line = update.state.doc.lineAt(head);
              latest.current.onCaret(line.number, head - line.from + 1);
            }
          }),
        ],
      }),
    });

    view.current = editor;
    editor.dispatch({ effects: [setError.of(latest.current.error), setAiMarks.of(latest.current.aiMarks ?? null)] });
    let flashTimer = 0;
    handle.current = {
      statementAtCaret: () => editor.state.field(statement),
      selectedStatements() {
        const { from, to } = editor.state.selection.main;
        if (from === to) return null;
        // Offsets are those of the whole text, so an error can be pointed at later.
        return splitFor(editor.state.sliceDoc(from, to), lang).map((s) => ({ ...s, from: s.from + from, to: s.to + from }));
      },
      text: () => editor.state.doc.toString(),
      replaceText: (text) => editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } }),
      goTo(place) {
        const line = editor.state.doc.line(Math.min(Math.max(1, place.line), editor.state.doc.lines));
        const anchor = place.offset != null ? Math.min(place.offset, editor.state.doc.length) : line.from;
        editor.dispatch({ selection: { anchor }, scrollIntoView: true });
        editor.focus();
      },
      flashLine(number) {
        const line = editor.state.doc.line(Math.min(Math.max(1, number), editor.state.doc.lines));
        editor.dispatch({ selection: { anchor: line.from }, scrollIntoView: true, effects: setFlash.of(line.number) });
        editor.focus();
        window.clearTimeout(flashTimer);
        flashTimer = window.setTimeout(() => editor.dispatch({ effects: setFlash.of(null) }), 200);
      },
    };
    editor.focus();
    return () => {
      window.clearTimeout(flashTimer);
      handle.current = null;
      view.current = null;
      editor.destroy();
    };
  }, []);

  return <div ref={host} className="selectable min-h-0 overflow-hidden" />;
}
