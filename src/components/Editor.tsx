// SQL editor: CodeMirror 6 themed from the --syn-* tokens (see index.css),
// with a gutter bar marking the statement that Run would execute.
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
import { t } from "../i18n";
import { usePrefs } from "../prefs";
import { splitStatements, statementAt, type SqlDialect, type Statement } from "../lib/sql";

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
  { tag: [tags.number, tags.bool, tags.null], color: "var(--syn-num)" },
  { tag: [tags.standard(tags.name), tags.function(tags.variableName)], color: "var(--syn-fn)" },
  { tag: [tags.lineComment, tags.blockComment], color: "var(--syn-com)", fontStyle: "italic" },
]);

const DIALECTS = { postgres: PostgreSQL, mysql: MySQL, sqlite: SQLite, sqlserver: MSSQL, clickhouse: MySQL, duckdb: PostgreSQL };

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
  dialect: SqlDialect;
  /** Tables and their columns, offered as completions. Tables without loaded columns map to []. */
  schema: Record<string, string[]>;
  initialText: string;
  error: ErrorMark | null;
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
  const languageFor = (dialect: SqlDialect, schema: Record<string, string[]>) =>
    sql({ dialect: DIALECTS[dialect], schema, upperCaseKeywords: true });

  useEffect(() => {
    view.current?.dispatch({ effects: language.current.reconfigure(languageFor(props.dialect, props.schema)) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.schema]);

  useEffect(() => {
    view.current?.dispatch({ effects: setError.of(props.error) });
  }, [props.error]);

  useEffect(() => {
    const { dialect, initialText, handle } = latest.current;
    // Settings › Editor, read when the editor is made.
    const prefs = usePrefs.getState();
    document.documentElement.style.setProperty("--completion-hint", JSON.stringify(t("↑↓ Move · ↵ Tab Accept · Esc Close")));
    const range = (state: EditorState) => statementAt(state.doc.toString(), state.selection.main.head, dialect);
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
          gutter({
            class: "cm-error-gutter",
            lineMarker: (view, line) => (view.state.field(errorField)?.line === view.state.doc.lineAt(line.from).number ? errorDiamond : null),
            lineMarkerChange: (update) => update.startState.field(errorField) !== update.state.field(errorField),
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
          placeholder(t("Write SQL here")),
          language.current.of(languageFor(dialect, latest.current.schema)),
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
          functionHighlight,
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
    editor.dispatch({ effects: setError.of(latest.current.error) });
    handle.current = {
      statementAtCaret: () => editor.state.field(statement),
      selectedStatements() {
        const { from, to } = editor.state.selection.main;
        if (from === to) return null;
        // Offsets are those of the whole text, so an error can be pointed at later.
        return splitStatements(editor.state.sliceDoc(from, to), dialect).map((s) => ({ ...s, from: s.from + from, to: s.to + from }));
      },
      text: () => editor.state.doc.toString(),
      replaceText: (text) => editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } }),
      goTo(place) {
        const line = editor.state.doc.line(Math.min(Math.max(1, place.line), editor.state.doc.lines));
        const anchor = place.offset != null ? Math.min(place.offset, editor.state.doc.length) : line.from;
        editor.dispatch({ selection: { anchor }, scrollIntoView: true });
        editor.focus();
      },
    };
    editor.focus();
    return () => {
      handle.current = null;
      view.current = null;
      editor.destroy();
    };
  }, []);

  return <div ref={host} className="selectable min-h-0 overflow-hidden" />;
}
