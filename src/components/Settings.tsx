// The Settings window (shell from G01, provisional) with the sections that
// exist so far: Appearance (theme and language) and AI (AI01–AI03).
import { ChevronDown, ChevronRight, Eye, EyeOff, Lock, Minus, Plus, Search, Sparkles, TriangleAlert } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { loadProviders, setPrivacy, useAi, usable } from "../ai-store";
import { ai, aiError, type ModelInfo, type ProviderEntry, type ProviderKind } from "../lib/ai";
import { LANGUAGES, msg, t } from "../i18n";
import { chooseLanguage, openDialog, setTheme, useApp, type SettingsSection, type ThemePref } from "../store";
import { groupDigits, setPref, usePrefs, type Prefs } from "../prefs";
import { inTauri } from "../lib/api";
import { Button, Dialog, Segmented, Strip, Switch, TextInput } from "./dialog";
import { Spinner } from "./Results";
import { Mark, NOT_BUILT, Tag } from "./ui";

type Section = SettingsSection;

const NAV: { id: Section; label: string; ai?: boolean }[] = [
  { id: "appearance", label: msg("Appearance") },
  { id: "editor", label: msg("Editor") },
  { id: "data", label: msg("Data") },
  { id: "shortcuts", label: msg("Keyboard shortcuts") },
  { id: "ai", label: "AI", ai: true },
  { id: "security", label: msg("Security") },
  { id: "about", label: msg("About") },
];

export function Settings({ section, aiTab }: { section: Section; aiTab?: "providers" | "privacy" }) {
  const go = (next: Section) => openDialog({ kind: "settings", section: next });
  return (
    <Dialog title={t("Settings")} width={980} offset={-40} bodyClassName="" footer={null} onClose={() => openDialog(null)}>
      <div className="grid h-[726px] max-h-[calc(100vh-140px)] grid-cols-[200px_minmax(0,1fr)]">
        <nav className="flex flex-col border-r border-line bg-surface py-s2">
          {NAV.map((item) => {
            const active = item.id === section;
            return (
              <button
                key={item.label}
                type="button"
                aria-current={active ? "page" : undefined}
                className={`flex h-[28px] items-center gap-s2 px-s4 text-left text-[13px] disabled:cursor-not-allowed disabled:text-text-3 ${active ? "bg-accent-tint font-medium" : "enabled:hover:bg-hover"}`}
                style={active ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
                onClick={() => go(item.id)}
              >
                {item.ai && <Sparkles size={13} strokeWidth={1.5} className="text-accent-text" />}
                {t(item.label)}
              </button>
            );
          })}
        </nav>
        <div className="min-h-0 overflow-auto">
          {section === "appearance" && <Appearance />}
          {section === "editor" && <EditorSection />}
          {section === "data" && <DataSection />}
          {section === "shortcuts" && <Shortcuts />}
          {section === "ai" && <AiSection tab={aiTab ?? "providers"} />}
          {section === "security" && <Security />}
          {section === "about" && <About />}
        </div>
      </div>
    </Dialog>
  );
}

function Row({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-s4 border-b border-line-2 py-s3">
      <div className="min-w-0 flex-1">
        <div className="font-medium">{title}</div>
        {description && <div className="text-[12.5px] text-text-2">{description}</div>}
      </div>
      {children}
    </div>
  );
}

const Group = ({ children }: { children: ReactNode }) => <div className="label flex items-center gap-s2 border-b border-line pt-s4 pb-s2">{children}</div>;

const select = "h-[28px] min-w-0 border border-line bg-panel px-s2 text-[12.5px] outline-none hover:border-text-3 focus:border-accent disabled:cursor-not-allowed disabled:opacity-45";

function Appearance() {
  const theme = useApp((s) => s.theme);
  const language = useApp((s) => s.language);
  const prefs = usePrefs();
  return (
    <div className="px-[24px] pb-s4">
      <Group>{t("Look")}</Group>
      <Row title={t("Theme")} description={t("System follows the operating system.")}>
        <Segmented
          label={t("Theme")}
          value={theme}
          onChange={(value: ThemePref) => setTheme(value)}
          options={[
            { value: "system", label: t("System") },
            { value: "light", label: t("Light") },
            { value: "dark", label: t("Dark") },
          ]}
        />
      </Row>
      <Row title={t("Language")} description={t("Each language is listed in its own name.")}>
        <select className={`${select} w-[200px]`} value={language} aria-label={t("Language")} onChange={(e) => chooseLanguage(e.target.value as typeof language)}>
          {LANGUAGES.map(({ code, name }) => (
            <option key={code} value={code}>
              {name}
            </option>
          ))}
        </select>
      </Row>
      <Row title={t("Density")} description={t("Row height in grids and lists.")}>
        <Segmented
          label={t("Density")}
          value={prefs.density}
          onChange={(value: Prefs["density"]) => setPref("density", value)}
          options={[
            { value: "compact", label: t("Compact") },
            { value: "comfortable", label: t("Comfortable") },
          ]}
        />
      </Row>
      <Row title={t("Editor font size")} description={t("11 to 20 pixels.")}>
        <Stepper value={prefs.codeSize} min={11} max={20} unit="px" onChange={(value) => setPref("codeSize", value)} />
      </Row>
      <Group>{t("Preview")}</Group>
      <div className="mt-s3 border border-line bg-panel">
        <pre className="border-b border-line-2 px-s3 py-s2 font-mono" style={{ fontSize: prefs.codeSize, lineHeight: `${prefs.codeSize + 7}px` }}>
          <span style={{ color: "var(--syn-kw)" }}>SELECT</span> id, total <span style={{ color: "var(--syn-kw)" }}>FROM</span> orders <span style={{ color: "var(--syn-kw)" }}>WHERE</span> status = <span style={{ color: "var(--syn-str)" }}>'paid'</span>;
        </pre>
        {[
          ["48213", "2184.50"],
          ["10977", "1967.00"],
        ].map(([id, total]) => (
          <div key={id} className="grid h-row grid-cols-[120px_120px_minmax(0,1fr)] items-center border-b border-line-2 font-mono text-[12.5px] last:border-b-0">
            <span className="px-[10px] text-right">{id}</span>
            <span className="px-[10px] text-right">{total}</span>
            <span />
          </div>
        ))}
      </div>
      <p className="pt-s3 text-[12.5px] text-text-3">{t("Interface font size and font choice are designed but not built yet.")}</p>
    </div>
  );
}

function Stepper({ value, min, max, unit, onChange }: { value: number; min: number; max: number; unit?: string; onChange(value: number): void }) {
  const button = "grid size-[28px] place-items-center text-text-2 enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
  return (
    <div className="flex h-[28px] items-center border border-line">
      <button type="button" aria-label={t("Decrease")} className={button} disabled={value <= min} onClick={() => onChange(value - 1)}>
        <Minus size={13} />
      </button>
      <span className="w-[56px] border-x border-line text-center font-mono text-[12.5px] leading-[26px]">
        {value}
        {unit ? ` ${unit}` : ""}
      </span>
      <button type="button" aria-label={t("Increase")} className={button} disabled={value >= max} onClick={() => onChange(value + 1)}>
        <Plus size={13} />
      </button>
    </div>
  );
}

const Toggle = ({ pref, title, description }: { pref: "wordWrap" | "autocomplete" | "uppercaseKeywords" | "highlightStatement" | "previewBeforeApply"; title: string; description: string }) => {
  const value = usePrefs((s) => s[pref]);
  return (
    <Row title={title} description={description}>
      <Switch checked={value} onChange={(checked) => setPref(pref, checked)}>
        <span className="sr-only">{title}</span>
      </Switch>
    </Row>
  );
};

/** A row for a setting that is designed but has nothing behind it yet. */
const Later = ({ title, description }: { title: string; description: string }) => (
  <div className="opacity-60" title={t(NOT_BUILT)}>
    <Row title={title} description={description}>
      <Tag>{t(NOT_BUILT)}</Tag>
    </Row>
  </div>
);

function EditorSection() {
  const tabWidth = usePrefs((s) => s.tabWidth);
  return (
    <div className="px-[24px] pb-s4">
      <Group>{t("Text")}</Group>
      <Row title={t("Tab width")} description={t("How wide a tab is, and how far Tab indents.")}>
        <Segmented
          label={t("Tab width")}
          value={String(tabWidth)}
          onChange={(value) => setPref("tabWidth", Number(value) as Prefs["tabWidth"])}
          options={[
            { value: "2", label: "2" },
            { value: "4", label: "4" },
            { value: "8", label: "8" },
          ]}
        />
      </Row>
      <Toggle pref="wordWrap" title={t("Word wrap")} description={t("Long lines continue on the next line instead of scrolling sideways.")} />
      <Group>{t("Help while typing")}</Group>
      <Toggle pref="autocomplete" title={t("Autocomplete")} description={t("Suggest keywords, tables and columns as you type.")} />
      <Toggle pref="highlightStatement" title={t("Highlight the statement under the cursor")} description={t("A bar in the gutter marks what Run will execute.")} />
      <Group>{t("Format")}</Group>
      <Toggle pref="uppercaseKeywords" title={t("Uppercase keywords")} description={t("Format writes SELECT, FROM and other keywords in capitals.")} />
      <p className="pt-s3 text-[12.5px] text-text-3">{t("Changes apply to an editor the next time its tab is opened.")}</p>
    </div>
  );
}

function DataSection() {
  const prefs = usePrefs();
  return (
    <div className="px-[24px] pb-s4">
      <Group>{t("Queries")}</Group>
      <Row title={t("Default row limit")} description={t("New query tabs start with this limit.")}>
        <Segmented
          label={t("Default row limit")}
          value={String(prefs.defaultLimit)}
          onChange={(value) => setPref("defaultLimit", Number(value))}
          options={[100, 500, 1000, 5000].map((n) => ({ value: String(n), label: String(n) }))}
        />
      </Row>
      <Later title={t("Ask before running without a limit")} description={t("Confirm a SELECT that would return every row.")} />
      <Group>{t("Display")}</Group>
      <Row title={t("Numbers")} description={t("Copying always gives the raw value.")}>
        <div className="flex items-center gap-s3">
          <span className="font-mono text-[12.5px] text-text-2">{prefs.numbers === "grouped" ? groupDigits("1284500.75") : "1284500.75"}</span>
          <Segmented
            label={t("Numbers")}
            value={prefs.numbers}
            onChange={(value: Prefs["numbers"]) => setPref("numbers", value)}
            options={[
              { value: "raw", label: t("Raw") },
              { value: "grouped", label: t("Grouped") },
            ]}
          />
        </div>
      </Row>
      <Row title="NULL" description={t("How a missing value is drawn in grids.")}>
        <div className="flex items-center gap-s3">
          <span className="grid h-[24px] w-[64px] place-items-center border border-line-2 font-mono text-[12px]">
            {prefs.nullStyle === "tag" ? <span className="border border-dashed border-line px-[4px] text-[11px] text-text-3 italic">NULL</span> : prefs.nullStyle === "dimmed" ? <span className="text-text-3">NULL</span> : ""}
          </span>
          <Segmented
            label="NULL"
            value={prefs.nullStyle}
            onChange={(value: Prefs["nullStyle"]) => setPref("nullStyle", value)}
            options={[
              { value: "tag", label: t("Tag") },
              { value: "dimmed", label: t("Dimmed text") },
              { value: "blank", label: t("Blank") },
            ]}
          />
        </div>
      </Row>
      <Later title={t("Date and time format")} description={t("Dates are shown as the database returns them.")} />
      <Later title={t("Time zone")} description={t("Times are shown as stored.")} />
    </div>
  );
}

const isMac = /Mac/i.test(navigator.userAgent);
const MOD = isMac ? "⌘" : "Ctrl+";
const ALT = isMac ? "⌥" : "Alt+";
const SHIFT = isMac ? "⇧" : "Shift+";
const ENTER = isMac ? "↵" : "Enter";
const SHORTCUTS: [string, string, string][] = [
  [msg("Run the statement at the cursor"), `${MOD}${ENTER}`, msg("Editor")],
  [msg("Run the selection"), `${SHIFT}${MOD}${ENTER}`, msg("Editor")],
  [msg("Stop the running statement"), `${MOD}.`, msg("Editor")],
  [msg("Explain"), `${MOD}E`, msg("Editor")],
  [msg("Save this query"), `${MOD}S`, msg("Editor")],
  [msg("Ask AI"), `${MOD}I`, msg("Editor")],
  [msg("Commit"), `${ALT}${MOD}${ENTER}`, msg("Editor")],
  [msg("Command palette"), `${MOD}K`, msg("Everywhere")],
  [msg("Show or hide the sidebar"), `${MOD}B`, msg("Everywhere")],
  [msg("New connection"), `${MOD}N`, msg("Everywhere")],
  [msg("New query tab"), `${MOD}T`, msg("Everywhere")],
  [msg("Close tab"), `${MOD}W`, msg("Editor")],
  [msg("Apply pending changes"), `${MOD}S`, msg("Table data")],
  [msg("Undo the last edit"), `${MOD}Z`, msg("Table data")],
  [msg("Set NULL"), `${MOD}⌫`, msg("Table data")],
  [msg("Edit the cell"), "F2", msg("Table data")],
];

function Shortcuts() {
  const [query, setQuery] = useState("");
  const rows = SHORTCUTS.filter(([command, keys]) => `${t(command)} ${keys}`.toLowerCase().includes(query.trim().toLowerCase()));
  return (
    <div className="px-[24px] pb-s4">
      <div className="flex items-center gap-s3 py-s3">
        <TextInput className="w-[280px]" value={query} placeholder={t("Search shortcuts")} aria-label={t("Search shortcuts")} onChange={(e) => setQuery(e.target.value)} before={<Search size={13} className="text-text-3" />} />
        <span className="font-mono text-[11.5px] text-text-2">
          {rows.length}/{SHORTCUTS.length}
        </span>
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)_160px_160px] border border-line">
        {[t("Command"), t("Shortcut"), t("Where")].map((header) => (
          <span key={header} className="label flex h-row items-center border-b border-line bg-bg px-[10px]">
            {header}
          </span>
        ))}
        {rows.map(([command, keys, where]) => (
          <div key={command + where} className="contents">
            <span className="flex h-row items-center border-b border-line-2 px-[10px] text-[13px]">{t(command)}</span>
            <span className="flex h-row items-center border-b border-line-2 px-[10px] font-mono text-[12px]">{keys}</span>
            <span className="flex h-row items-center border-b border-line-2 px-[10px] text-[12.5px] text-text-2">{t(where)}</span>
          </div>
        ))}
      </div>
      <p className="pt-s3 text-[12.5px] text-text-3">{t("Changing shortcuts is designed but not built yet.")}</p>
    </div>
  );
}

function Security() {
  const locked = (
    <span className="flex items-center gap-[6px] text-[12.5px] text-text-2">
      <Lock size={12} />
      {t("Always on")}
    </span>
  );
  return (
    <div className="px-[24px] pb-s4">
      <Group>
        {t("Production")}
        <Tag kind="warn">PROD</Tag>
      </Group>
      <Row title={t("Confirm destructive statements")} description={t("DROP, TRUNCATE, DELETE and UPDATE without WHERE ask before they run.")}>
        {locked}
      </Row>
      <Row title={t("Type the connection name to confirm")} description={t("The button stays disabled until the name matches.")}>
        {locked}
      </Row>
      <Row title={t("Preview SQL before applying data changes")} description={t("On Production the statements are always shown first.")}>
        {locked}
      </Row>
      <Group>{t("Other connections")}</Group>
      <Toggle pref="previewBeforeApply" title={t("Preview SQL before applying data changes")} description={t("Show the statements first on Staging and Local too.")} />
      <Group>{t("Passwords")}</Group>
      <Row title={t("Passwords are saved in the system keychain")} description={t("Chosen per connection in its form. They are never written to the settings file.")}>
        {locked}
      </Row>
      <Later title={t("Remove all saved passwords")} description={t("Deletes every password this app stored in the keychain.")} />
      <Group>SSH</Group>
      <Later title={t("known_hosts file and unknown hosts")} description={t("Set per connection in its SSH tab for now.")} />
    </div>
  );
}

function About() {
  const [version, setVersion] = useState("0.1.0");
  useEffect(() => {
    if (inTauri) void import("@tauri-apps/api/app").then((app) => app.getVersion()).then(setVersion).catch(() => {});
  }, []);
  const system = [`LewIDE ${version}`, navigator.userAgent, `${t("Language")}: ${navigator.language}`, `${window.screen.width}×${window.screen.height} @${window.devicePixelRatio}x`].join("\n");
  return (
    <div className="flex flex-col gap-s4 px-[24px] py-s6">
      <div className="flex items-center gap-[14px]">
        <Mark size={56} />
        <div className="flex flex-col gap-[4px]">
          <span className="font-heading text-[34px] leading-none font-semibold tracking-[.01em]">
            Lew<span className="text-accent-text">IDE</span>
          </span>
          <span className="selectable font-mono text-[11.5px] text-text-3">{t("Version {version}", { version })}</span>
        </div>
      </div>
      <p className="max-w-[560px] leading-[1.5] text-text-2">{t("A database client for developers and data teams. Open source.")}</p>
      <div>
        <Group>{t("System")}</Group>
        <pre className="selectable mt-s3 border border-line bg-bg p-s3 font-mono text-[12px] leading-[20px] whitespace-pre-wrap">{system}</pre>
        <div className="pt-s2">
          <Button onClick={() => void navigator.clipboard?.writeText(system)}>{t("Copy")}</Button>
        </div>
      </div>
      <p className="text-[12.5px] text-text-3">{t("Update checks are designed but not built yet.")}</p>
    </div>
  );
}

const KINDS: { kind: ProviderKind; name: string }[] = [
  { kind: "anthropic", name: "Anthropic" },
  { kind: "open_ai", name: "OpenAI" },
  { kind: "gemini", name: "Google Gemini" },
];

function AiSection({ tab }: { tab: "providers" | "privacy" }) {
  const providers = useAi((s) => s.providers);
  const loaded = useAi((s) => s.loaded);
  // Blocks opened for a provider that is not saved yet.
  const [drafts, setDrafts] = useState<{ key: number; kind: ProviderKind }[]>([]);
  useEffect(() => void loadProviders(), []);
  const setTab = (aiTab: "providers" | "privacy") => openDialog({ kind: "settings", section: "ai", aiTab });
  const addDraft = (kind: ProviderKind) => setDrafts((d) => [...d, { key: Date.now(), kind }]);
  const empty = loaded && providers.length === 0 && drafts.length === 0;
  const ready = providers.filter(usable);
  const fallback = ready.find((p) => p.is_default) ?? null;

  return (
    <div className="flex min-h-full flex-col">
      <div role="tablist" className="flex h-[34px] flex-none items-stretch gap-[20.4px] border-b border-line px-[20px]">
        {(["providers", "privacy"] as const).map((id) => (
          <button key={id} type="button" role="tab" aria-selected={tab === id} className={`-mb-px border-b-2 text-[13px] ${tab === id ? "border-accent font-medium" : "border-transparent text-text-2 hover:text-text"}`} onClick={() => setTab(id)}>
            {id === "providers" ? t("Providers") : t("Privacy")}
          </button>
        ))}
      </div>

      {tab === "privacy" ? (
        <PrivacyTab />
      ) : empty ? (
        // AI02
        <div className="grid flex-1 place-items-center">
          <div className="flex w-[520px] max-w-full flex-col items-start gap-s3 px-s4">
            <Sparkles size={24} strokeWidth={1.5} className="text-accent-text" />
            <h3 className="font-heading text-[24px] leading-[1.1] font-semibold">{t("No AI provider yet")}</h3>
            <p className="leading-[1.5] text-text-2">{t("Add a key from a provider you already use. The assistant explains, checks and improves SQL; it never runs anything.")}</p>
            <div className="grid w-full grid-cols-2 gap-s2">
              {[...KINDS, { kind: "open_ai_compatible" as ProviderKind, name: t("OpenAI-compatible endpoint") }].map((item, i) => (
                <button key={item.kind} type="button" data-autofocus={i === 0 ? true : undefined} className="flex h-[36px] items-center gap-s2 border border-line bg-panel px-s3 hover:bg-hover" onClick={() => addDraft(item.kind)}>
                  <Plus size={13} className="text-accent-text" />
                  {item.name}
                </button>
              ))}
            </div>
            <p className="text-[12.5px] text-text-2">{t("Keys are stored in the system keychain, not in the app's settings file.")}</p>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-[10.2px] px-[20px] py-[13.6px]">
          <label className="flex w-[240px] flex-col gap-[4px]">
            <span className="text-[12px] text-text-2">{t("Default provider")}</span>
            <select
              className={select}
              value={fallback?.id ?? ""}
              disabled={ready.length === 0}
              onChange={(e) => void ai.setDefaultProvider(e.target.value).then(loadProviders)}
            >
              {ready.length === 0 && <option value="">{t("No provider")}</option>}
              {ready.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                  {p.model ? ` · ${p.model}` : ""}
                </option>
              ))}
            </select>
          </label>

          {providers.map((provider) => (
            <ProviderBlock key={provider.id} provider={provider} kind={provider.kind} />
          ))}
          {drafts.map((draft) => (
            <ProviderBlock key={draft.key} provider={null} kind={draft.kind} onSaved={() => setDrafts((d) => d.filter((x) => x.key !== draft.key))} onCancel={() => setDrafts((d) => d.filter((x) => x.key !== draft.key))} />
          ))}
          {/* The three known providers stay listed until they are set up. */}
          {KINDS.filter((k) => !providers.some((p) => p.kind === k.kind) && !drafts.some((d) => d.kind === k.kind)).map((k) => (
            <div key={k.kind} className="flex h-[36px] items-center gap-s3 border border-line px-s3">
              <ChevronRight size={13} className="text-text-3" />
              <span className="font-medium">{k.name}</span>
              <span className="ml-auto text-[12.5px] text-text-3">{t("Not set up")}</span>
              <button type="button" className="h-[24px] border border-line bg-panel px-s2 text-[12.5px] hover:bg-hover" onClick={() => addDraft(k.kind)}>
                {t("Add key")}
              </button>
            </div>
          ))}
          <div className="flex items-center gap-s3">
            <button type="button" className="flex h-[26px] items-center gap-[6px] px-s2 text-accent-text hover:bg-hover" onClick={() => addDraft("open_ai_compatible")}>
              <Plus size={13} />
              {t("Add OpenAI-compatible endpoint")}
            </button>
            <span className="text-[12.5px] text-text-3">{t("Ollama, OpenRouter, Groq, Mistral and similar services")}</span>
          </div>
        </div>
      )}
    </div>
  );
}

type Test = { state: "idle" } | { state: "running" } | { state: "ok"; ms: number; models: number } | { state: "failed"; message: string };

function ProviderBlock({ provider, kind, onSaved, onCancel }: { provider: ProviderEntry | null; kind: ProviderKind; onSaved?(): void; onCancel?(): void }) {
  const compatible = kind === "open_ai_compatible";
  const [open, setOpen] = useState(true);
  const [name, setName] = useState(provider?.name ?? (compatible ? "" : KINDS.find((k) => k.kind === kind)!.name));
  const [baseUrl, setBaseUrl] = useState(provider?.base_url ?? (compatible ? "http://localhost:11434/v1" : ""));
  const [key, setKey] = useState("");
  const [replacing, setReplacing] = useState(!provider?.has_key);
  const [shown, setShown] = useState(false);
  const [model, setModel] = useState(provider?.model ?? "");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [test, setTest] = useState<Test>({ state: "idle" });
  const [removing, setRemoving] = useState(false);
  const local = compatible && /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])([:/]|$)/i.test(baseUrl);
  const hasKey = !!provider?.has_key && !replacing;
  const ready = !!provider && usable(provider) && test.state !== "failed";
  const canTest = test.state !== "running" && name.trim() !== "" && (compatible ? baseUrl.trim() !== "" : hasKey || key.trim() !== "");

  // The models of a provider that already works, for the select.
  useEffect(() => {
    if (provider && usable(provider)) void ai.listModels(provider.id).then(setModels).catch(() => {});
  }, [provider?.id, provider?.has_key]); // eslint-disable-line react-hooks/exhaustive-deps

  async function runTest() {
    setTest({ state: "running" });
    const started = performance.now();
    const typed = replacing && key.trim() !== "" ? key.trim() : undefined;
    let saved: ProviderEntry | null = null;
    try {
      saved = await ai.saveProvider({ id: provider?.id ?? "", kind, name: name.trim(), base_url: compatible ? baseUrl.trim() : null, model: model || null }, typed);
      const list = await ai.listModels(saved.id).catch(() => [] as ModelInfo[]);
      setModels(list);
      let chosen = model;
      // Every provider but Anthropic needs a model before it can answer.
      if (!chosen && kind !== "anthropic" && list.length > 0) {
        chosen = list[0].id;
        setModel(chosen);
        saved = await ai.saveProvider({ ...saved, model: chosen });
      }
      await ai.testProvider(saved.id);
      setTest({ state: "ok", ms: Math.round(performance.now() - started), models: list.length });
      setKey("");
      setReplacing(false);
      await loadProviders();
      onSaved?.();
    } catch (e) {
      // A provider being added that fails its first test is not kept: each
      // try would otherwise leave another empty row. An existing one only
      // loses the key that does not work.
      if (saved && !provider) await ai.deleteProvider(saved.id).catch(() => {});
      else if (saved && typed !== undefined) await ai.saveProvider(saved, "").catch(() => {});
      await loadProviders();
      setTest({ state: "failed", message: aiError(e).message });
    }
  }

  async function chooseModel(next: string) {
    setModel(next);
    if (provider) await ai.saveProvider({ ...provider, model: next || null }).then(loadProviders).catch(() => {});
  }

  const status = test.state === "failed" ? (
    <span className="flex items-center gap-[6px] text-danger">
      <TriangleAlert size={12} />
      {compatible ? t("Not reachable") : t("Key rejected")}
    </span>
  ) : ready ? (
    <span className="text-text-2">{t("Ready")}</span>
  ) : (
    <span className="text-text-3">{t("Not set up")}</span>
  );

  return (
    <div className="border border-line">
      <div className="flex h-[36px] items-center gap-s3 px-s3">
        <button type="button" aria-expanded={open} aria-label={name} className="grid size-[18px] place-items-center text-text-2 hover:bg-hover" onClick={() => setOpen(!open)}>
          {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        </button>
        <span className="truncate font-medium">{name || t("New endpoint")}</span>
        {compatible && <span className="text-[12.5px] text-text-2">{t("OpenAI-compatible endpoint")}</span>}
        {local && <Tag>{t("On this device")}</Tag>}
        {provider?.is_default && <Tag kind="accent">{t("Default")}</Tag>}
        <span className="ml-auto text-[12.5px]">{status}</span>
      </div>
      {open && (
        <div className="flex flex-col gap-[10.2px] border-t border-line-2 p-[10.2px]">
          <div className={`grid gap-s3 ${compatible ? "grid-cols-[160px_minmax(0,1fr)_200px_200px]" : "grid-cols-[minmax(0,1fr)_240px]"}`}>
            {compatible && (
              <>
                <label className="flex min-w-0 flex-col gap-[4px]">
                  <span className="text-[12px] text-text-2">{t("Name")}</span>
                  <TextInput value={name} onChange={(e) => setName(e.target.value)} />
                </label>
                <label className="flex min-w-0 flex-col gap-[4px]">
                  <span className="text-[12px] text-text-2">{t("Base URL")}</span>
                  <TextInput mono value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
                </label>
              </>
            )}
            <label className="flex min-w-0 flex-col gap-[4px]">
              <span className="text-[12px] text-text-2">{t("API key")}</span>
              {hasKey ? (
                <span className="flex h-[28px] items-center gap-s2 text-[12.5px] text-text-3">
                  <Lock size={12} />
                  <span className="truncate">{t("Saved in system keychain · not shown")}</span>
                  <button type="button" className="text-accent-text hover:underline" onClick={() => setReplacing(true)}>
                    {t("Replace")}
                  </button>
                </span>
              ) : (
                <TextInput
                  mono
                  type={shown ? "text" : "password"}
                  autoComplete="off"
                  value={key}
                  invalid={test.state === "failed" && !compatible}
                  placeholder={compatible ? t("None · optional") : undefined}
                  onChange={(e) => setKey(e.target.value)}
                  after={
                    <button type="button" aria-label={shown ? t("Hide key") : t("Show key")} title={shown ? t("Hide key") : t("Show key")} className="grid size-[20px] place-items-center text-text-2 hover:bg-hover" onClick={() => setShown(!shown)}>
                      {shown ? <EyeOff size={13} /> : <Eye size={13} />}
                    </button>
                  }
                />
              )}
              {!hasKey && !compatible && <span className="text-[11.5px] text-text-3">{t("Stored in the system keychain when the test succeeds.")}</span>}
            </label>
            <label className="flex min-w-0 flex-col gap-[4px]">
              <span className="text-[12px] text-text-2">{t("Model")}</span>
              <select className={`${select} font-mono`} value={model} disabled={models.length === 0} onChange={(e) => void chooseModel(e.target.value)}>
                {models.length === 0 ? (
                  <option value={model}>{model || t("Test the key to load models")}</option>
                ) : (
                  <>
                    {kind === "anthropic" && <option value="">{t("Provider default")}</option>}
                    {model && !models.some((m) => m.id === model) && <option value={model}>{model}</option>}
                    {models.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.id}
                      </option>
                    ))}
                  </>
                )}
              </select>
            </label>
          </div>

          <div className="flex min-h-[28px] items-center gap-s3 text-[12.5px]">
            <Button disabled={!canTest} onClick={runTest}>
              {t("Test")}
            </Button>
            {test.state === "running" && (
              <>
                <Spinner />
                <span>{t("Testing…")}</span>
              </>
            )}
            {test.state === "ok" && (
              <>
                <i className="size-[9px] flex-none bg-ok" />
                <span>{compatible ? t("Reachable") : t("Key works")}</span>
                <span className="font-mono text-[12px] text-text-2">{t("{ms} ms · {count} models available", { ms: test.ms, count: test.models })}</span>
              </>
            )}
            {test.state === "failed" && (
              <>
                <TriangleAlert size={13} className="flex-none text-danger" />
                <span className="flex-none">{t("Test failed")}</span>
                <span className="selectable min-w-0 font-mono text-[12px] break-words">{test.message}</span>
              </>
            )}
            {local && test.state !== "failed" && <span className="text-text-2">{t("Requests to this endpoint stay on this computer.")}</span>}
            <span className="flex-1" />
            {provider ? (
              removing ? (
                // Not drawn in the design: confirmed in place.
                <>
                  <span className="text-text-2">{t("Remove this provider and its key?")}</span>
                  <button type="button" className="text-danger hover:underline" onClick={() => void ai.deleteProvider(provider.id).then(loadProviders)}>
                    {t("Remove")}
                  </button>
                  <button type="button" className="text-accent-text hover:underline" onClick={() => setRemoving(false)}>
                    {t("Cancel")}
                  </button>
                </>
              ) : (
                <button type="button" className="text-danger hover:underline" onClick={() => setRemoving(true)}>
                  {t("Remove")}
                </button>
              )
            ) : (
              <button type="button" className="text-accent-text hover:underline" onClick={onCancel}>
                {t("Cancel")}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function SwitchRow({ title, description, checked, locked, onChange }: { title: string; description: string; checked: boolean; locked?: string; onChange?(checked: boolean): void }) {
  return (
    <label className={`flex items-start gap-s3 border-b border-line-2 py-s3 ${locked ? "opacity-60" : "cursor-pointer"}`} title={locked}>
      <span className={`relative mt-[2px] h-[16px] w-[28px] flex-none border ${checked ? "border-accent-fill bg-accent-fill" : "border-text-2"}`}>
        <input type="checkbox" role="switch" checked={checked} disabled={!!locked} tabIndex={locked ? -1 : 0} onChange={(e) => onChange?.(e.target.checked)} className="absolute inset-0 cursor-pointer opacity-0 disabled:cursor-not-allowed" />
        <i className={`pointer-events-none absolute top-[1px] size-[12px] ${checked ? "right-[1px] bg-on-accent" : "left-[1px] bg-text-2"}`} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-s2">
          <span className="font-medium">{title}</span>
          <span className="text-[12px] text-text-2">{locked ? t("Locked") : checked ? t("On") : t("Off")}</span>
        </span>
        <span className="block text-[12.5px] text-text-2">{description}</span>
      </span>
    </label>
  );
}

function PrivacyTab() {
  const privacy = useAi((s) => s.privacy);
  // Sample rows and results are never sent: the core has no way to send them.
  const never = t("Not built: rows are never sent.");
  return (
    <div className="px-[20px] pb-s4">
      <Group>{t("What is sent with a request")}</Group>
      <SwitchRow title={t("Send schema")} description={t("Names of tables, columns, types and indexes. No values.")} checked={privacy.sendSchema} onChange={(sendSchema) => setPrivacy({ sendSchema })} />
      <SwitchRow title={t("Send sample rows")} description={t("Up to 5 rows per table in context.")} checked={false} locked={never} />
      <SwitchRow title={t("Send query results")} description={t("Rows of the current result when you ask about it.")} checked={false} locked={never} />
      <p className="py-s3 text-[12.5px] text-text-2">{t("Always sent: your question and the statement you are working on. Never sent: passwords, hosts and other connection details.")}</p>
      <Group>
        {t("On Production connections")}
        <Tag kind="warn">PROD</Tag>
      </Group>
      <SwitchRow title={t("Send schema")} description={t("Can be turned off for Production only.")} checked={privacy.sendSchema && privacy.productionSendSchema} locked={privacy.sendSchema ? undefined : t("Off for every connection above.")} onChange={(productionSendSchema) => setPrivacy({ productionSendSchema })} />
      <SwitchRow title={t("Send sample rows")} description={t("Never on Production.")} checked={false} locked={t("Locked on Production.")} />
      <SwitchRow title={t("Send query results")} description={t("Never on Production.")} checked={false} locked={t("Locked on Production.")} />
      <Strip kind="neutral" className="mt-s4">
        <span className="font-medium">{t("Local models.")}</span> {t("With an endpoint on this device, such as Ollama, requests do not leave this computer. The switches above still decide what the model sees.")}
      </Strip>
    </div>
  );
}
