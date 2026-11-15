import { useEffect, useMemo, useRef, useState } from "react";
import { archiveHistory, asUiError, FORMAT_LABEL, peekFile, type ArchiveInfo, type Entry, type Version } from "../api";
import type { Lang, makeT } from "../i18n";
import { baseName, human, mimeFor, num, previewKind } from "../util";
import { fileIcon, Icon } from "./Icon";
import { HealthPanel, type ViewCtx } from "./Views";

interface Props {
  t: ReturnType<typeof makeT>;
  lang: Lang;
  info: ArchiveInfo;
  password?: string;
  skipPrograms: boolean;
  setSkipPrograms: (v: boolean) => void;
  onExtract: (only: string[], chooseDest: boolean) => void;
  onClose: () => void;
  ctx: ViewCtx;
  onShare: () => void;
  onUpdate: () => void;
  onConvert: () => void;
  /** Bumped after an update so the history reloads. */
  revision: number;
  cwd: string;
  setCwd: (c: string) => void;
  editable: boolean;
  onOpenEntry: (e: Entry) => void;
  onDelete: (names: string[]) => void;
  onRename: (name: string) => void;
  onDragOut: (names: string[]) => void;
}

type Row =
  | { kind: "folder"; name: string; label: string; files: number; size: number }
  | { kind: "file"; name: string; label: string; entry: Entry };

type PeekState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "image"; url: string }
  | { kind: "pdf"; url: string }
  | { kind: "text"; text: string }
  | { kind: "none" }
  | { kind: "folder"; files: number; size: number }
  | { kind: "error"; message: string };

const HIDDEN = ".zipkoo/";
const clean = (n: string) => n.replace(/^\.\//, "");
const parentOf = (n: string) => {
  const t = n.endsWith("/") ? n.slice(0, -1) : n;
  const i = t.lastIndexOf("/");
  return i < 0 ? "" : t.slice(0, i + 1);
};
const leafOf = (n: string) => {
  const t = n.endsWith("/") ? n.slice(0, -1) : n;
  return t.slice(t.lastIndexOf("/") + 1);
};

/** Folder tree from a flat entry list. Archives often omit folder entries, so they're inferred. */
function buildTree(entries: Entry[]) {
  const files = new Map<string, Entry[]>();
  const folders = new Map<string, Set<string>>();
  const stats = new Map<string, { files: number; size: number }>();
  const addFolder = (dir: string) => {
    let d = dir;
    while (d) {
      const p = parentOf(d);
      if (!folders.has(p)) folders.set(p, new Set());
      folders.get(p)!.add(d);
      d = p;
    }
  };
  for (const e of entries) {
    const n = clean(e.name);
    if (!n || n.startsWith(HIDDEN)) continue;
    if (e.is_dir || n.endsWith("/")) {
      addFolder(n.endsWith("/") ? n : n + "/");
      continue;
    }
    const p = parentOf(n);
    if (!files.has(p)) files.set(p, []);
    files.get(p)!.push({ ...e, name: n });
    addFolder(p);
    for (let d = p; ; d = parentOf(d)) {
      const s = stats.get(d) ?? { files: 0, size: 0 };
      s.files += 1;
      s.size += e.size;
      stats.set(d, s);
      if (!d) break;
    }
  }
  return { files, folders, stats };
}

export function ArchiveView(props: Props) {
  const { t, lang, info, password, skipPrograms, setSkipPrograms, onExtract, onClose, ctx, onShare, onUpdate, onConvert, revision, cwd, setCwd, editable } = props;
  const [tab, setTab] = useState<"preview" | "history" | "health">("preview");
  const [drawer, setDrawer] = useState(false);
  const [versions, setVersions] = useState<Version[] | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<Row | null>(null);
  const [peek, setPeek] = useState<PeekState>({ kind: "idle" });
  const isZip = info.format === "zip";

  useEffect(() => {
    if (tab !== "history" || !isZip) return;
    archiveHistory(info.path).then((m) => setVersions(m.versions)).catch(() => setVersions([]));
  }, [tab, info.path, isZip, revision]);

  const tree = useMemo(() => buildTree(info.entries), [info.entries]);
  // A folder that vanished (renamed/deleted) sends you back to the nearest one that exists.
  useEffect(() => {
    let c = cwd;
    while (c && !tree.folders.has(c) && !tree.files.has(c)) c = parentOf(c);
    if (c !== cwd) setCwd(c);
  }, [tree, cwd, setCwd]);

  const rows: Row[] = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q) {
      const out: Row[] = [];
      for (const list of tree.files.values()) for (const e of list) if (e.name.toLowerCase().includes(q)) out.push({ kind: "file", name: e.name, label: leafOf(e.name), entry: e });
      return out.sort((a, b) => a.name.localeCompare(b.name));
    }
    const folders = [...(tree.folders.get(cwd) ?? [])].sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
    const files = [...(tree.files.get(cwd) ?? [])].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
    return [
      ...folders.map((f): Row => ({ kind: "folder", name: f, label: leafOf(f), ...(tree.stats.get(f) ?? { files: 0, size: 0 }) })),
      ...files.map((e): Row => ({ kind: "file", name: e.name, label: leafOf(e.name), entry: e })),
    ];
  }, [tree, cwd, query]);

  // Reset selection when the folder changes.
  useEffect(() => {
    setSelected(new Set());
    setFocus(null);
  }, [cwd]);

  const saved = info.total_size > 0 ? Math.max(0, Math.round(100 - (info.archive_size / info.total_size) * 100)) : 0;

  useEffect(() => {
    let url: string | null = null;
    let live = true;
    if (!focus) return setPeek({ kind: "idle" });
    if (focus.kind === "folder") return setPeek({ kind: "folder", files: focus.files, size: focus.size });
    const e = focus.entry;
    const kind = previewKind(e.name);
    if (kind === "none") return setPeek({ kind: "none" });
    if (e.size > 64 * 1024 * 1024) return setPeek({ kind: "error", message: t("peekTooBig") });
    setPeek({ kind: "loading" });
    peekFile(info.path, e.name, password)
      .then((bytes) => {
        if (!live) return;
        if (kind === "text") setPeek({ kind: "text", text: new TextDecoder().decode(bytes.slice(0, 64 * 1024)) });
        else {
          url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mimeFor(e.name) }));
          setPeek({ kind, url });
        }
      })
      .catch((err) => live && setPeek({ kind: "error", message: asUiError(err).message }));
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [focus, info.path, password, t]);

  const toggle = (name: string) => {
    const next = new Set(selected);
    next.has(name) ? next.delete(name) : next.add(name);
    setSelected(next);
  };
  const activate = (r: Row) => (r.kind === "folder" ? (setQuery(""), setCwd(r.name)) : props.onOpenEntry(r.entry));
  const targets = (r?: Row) => (r && !selected.has(r.name) ? [r.name] : [...selected]);

  // Only the rows on screen are rendered, so archives with 100,000+ entries scroll smoothly.
  const ROW = 56;
  const bodyRef = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 800 });
  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setView((v) => ({ ...v, height: el.clientHeight })));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    bodyRef.current?.scrollTo({ top: 0 });
    setView((v) => ({ ...v, top: 0 }));
  }, [cwd, query]);
  const first = Math.max(0, Math.floor(view.top / ROW) - 8);
  const last = Math.min(rows.length, Math.ceil((view.top + view.height) / ROW) + 8);
  const visible = rows.slice(first, last);

  // Drag rows out of the window (to Explorer/Finder): start once the pointer moves a little.
  const press = useRef<{ x: number; y: number; row: Row } | null>(null);
  const onPointerMove = (ev: React.PointerEvent) => {
    const p = press.current;
    if (!p || !(ev.buttons & 1)) return;
    if (Math.abs(ev.clientX - p.x) + Math.abs(ev.clientY - p.y) > 8) {
      press.current = null;
      props.onDragOut(targets(p.row));
    }
  };

  const onKeyDown = (ev: React.KeyboardEvent) => {
    if ((ev.target as HTMLElement).tagName === "INPUT") return;
    if (ev.key === "Backspace" && cwd) {
      ev.preventDefault();
      setCwd(parentOf(cwd));
    } else if ((ev.key === "Delete" || (ev.key === "Backspace" && ev.metaKey)) && editable && (selected.size || focus)) {
      ev.preventDefault();
      props.onDelete(targets(focus ?? undefined));
    } else if (ev.key === "F2" && editable && focus) {
      ev.preventDefault();
      props.onRename(focus.name);
    }
  };

  const progs = info.executables.length;
  const n = (v: number) => num(v, lang);
  const crumbs = cwd ? cwd.slice(0, -1).split("/") : [];
  const selCount = selected.size;
  const single = selCount === 1 ? [...selected][0] : focus && selCount === 0 ? focus.name : null;

  return (
    <div className={`archive ${drawer ? "drawer-open" : ""}`} onKeyDown={onKeyDown}>
      <section className="main-col" aria-label={baseName(info.path)}>
        <header className="ahead">
          <div className="ahead-text">
            <h1 className="title" title={info.path}>{baseName(info.path)}</h1>
            <p className="summary">
              {t("summary", { files: n(info.file_count), total: human(info.total_size, lang), packed: human(info.archive_size, lang) })}
              {saved > 0 && <span className="gain">{t("smaller", { pct: n(saved) })}</span>}
            </p>
          </div>
          <ul className="facts">
            <li className="fact"><Icon name="pack" size={16} />{t("formatLabel", { fmt: FORMAT_LABEL[info.format] })}</li>
            {info.encrypted && <li className="fact trust"><Icon name="lock" size={16} />{t("lockedNote")}</li>}
            <li className="fact trust"><Icon name="check" size={16} />{t("openedOk")}</li>
          </ul>
        </header>

        {progs > 0 && (
          <div className="notice" role="alert">
            <Icon name="shield" size={20} className="notice-icon" />
            <p>{t(progs === 1 ? "programsWarn" : "programsWarnMany", { n: n(progs) })}</p>
            <label className="switch">
              <input id="skip-programs" type="checkbox" role="switch" checked={skipPrograms} onChange={(e) => setSkipPrograms(e.target.checked)} />
              <span className="track" aria-hidden="true" />
              {t("programsAsk")}
            </label>
          </div>
        )}

        <div className="toolbar">
          <label className="search">
            <Icon name="search" size={17} />
            <input id="archive-search" placeholder={t("search")} value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          <div className="tool-buttons">
            <button className="btn icon-btn narrow-only" onClick={() => setDrawer(true)} title={t("previewTitle")} aria-label={t("previewTitle")} aria-expanded={drawer}><Icon name="image" size={18} /></button>
            <button className="btn icon-btn" onClick={onShare} title={t("shareLink")} aria-label={t("shareLink")}><Icon name="lock" size={18} /></button>
            {isZip && <button className="btn icon-btn" onClick={onUpdate} title={`${t("updateArchive")} ${t("updateHint")}`} aria-label={t("updateArchive")}><Icon name="pack" size={18} /></button>}
            <button className="btn icon-btn" onClick={onConvert} title={t("convertThis")} aria-label={t("convertThis")}><Icon name="zip" size={18} /></button>
          </div>
        </div>

        {!query && (
          <nav className="crumbs" aria-label="Folder">
            <button className="crumb-up" onClick={() => setCwd(parentOf(cwd))} disabled={!cwd} aria-label={t("goUp")} title={t("goUp")}><Icon name="extract" size={16} className="up" /></button>
            <button className={`crumb ${!cwd ? "here" : ""}`} onClick={() => setCwd("")}>{t("rootLabel")}</button>
            {crumbs.map((c, i) => {
              const path = crumbs.slice(0, i + 1).join("/") + "/";
              return (
                <span key={path} className="crumb-wrap">
                  <span className="crumb-sep" aria-hidden="true">›</span>
                  <button className={`crumb ${path === cwd ? "here" : ""}`} onClick={() => setCwd(path)}>{c}</button>
                </span>
              );
            })}
          </nav>
        )}

        <div className="list" role="table" aria-label={baseName(info.path)}>
          <div className="list-head" role="row">
            <span role="columnheader" className="c-check" />
            <span role="columnheader">{t("name")}</span>
            <span role="columnheader" className="c-size">{t("size")}</span>
            <span role="columnheader" className="c-size">{t("packed")}</span>
          </div>
          <div className="list-body" ref={bodyRef} onScroll={(ev) => { const top = ev.currentTarget.scrollTop; setView((v) => ({ ...v, top })); }} onPointerMove={onPointerMove} onPointerUp={() => (press.current = null)}>
            {rows.length === 0 && <p className="empty">{query ? t("noMatch", { q: query }) : t("noArchivesHere")}</p>}
            <div style={{ height: first * ROW }} aria-hidden="true" />
            {visible.map((r) => {
              const fi = r.kind === "folder" ? fileIcon(r.name, true) : fileIcon(r.name, false);
              const e = r.kind === "file" ? r.entry : null;
              return (
                <div
                  key={r.name}
                  role="row"
                  tabIndex={0}
                  aria-selected={focus?.name === r.name}
                  className={`row-item ${focus?.name === r.name ? "focus" : ""} ${selected.has(r.name) ? "picked" : ""}`}
                  onClick={() => setFocus(r)}
                  onDoubleClick={() => activate(r)}
                  onPointerDown={(ev) => { if (ev.button === 0 && !(ev.target as HTMLElement).closest("input")) press.current = { x: ev.clientX, y: ev.clientY, row: r }; }}
                  onKeyDown={(ev) => {
                    if (ev.key === " ") { ev.preventDefault(); toggle(r.name); }
                    if (ev.key === "Enter") { setFocus(r); activate(r); }
                  }}
                >
                  <span className="c-check" role="cell">
                    <input type="checkbox" aria-label={r.label} checked={selected.has(r.name)} onChange={() => toggle(r.name)} onClick={(ev) => ev.stopPropagation()} />
                  </span>
                  <span className="c-name" role="cell">
                    <span className={`ficon ${fi.tint}`}><Icon name={fi.icon} size={18} /></span>
                    <span className="names">
                      <span className="leaf">{r.label}</span>
                      {r.kind === "folder" ? (
                        <span className="path">{t("folderSummary", { files: n(r.files), size: human(r.size, lang) })}</span>
                      ) : query && parentOf(r.name) ? (
                        <span className="path">{parentOf(r.name).slice(0, -1)}</span>
                      ) : null}
                    </span>
                    {e?.executable && <span className="flag">{lang === "bn" ? "প্রোগ্রাম" : "Program"}</span>}
                  </span>
                  <span className="c-size" role="cell">{r.kind === "folder" ? human(r.size, lang) : human(e!.size, lang)}</span>
                  <span className="c-size dim" role="cell">
                    {!e || e.packed == null ? "" : e.packed >= e.size && e.size > 0 ? t("stored") : human(e.packed, lang)}
                  </span>
                </div>
              );
            })}
            <div style={{ height: (rows.length - last) * ROW }} aria-hidden="true" />
          </div>
        </div>

        <footer className="actions">
          <button className="btn primary" onClick={() => onExtract([...selected], false)}>
            <Icon name="extract" size={18} />
            {selCount ? `${t("extractSelected")} (${n(selCount)})` : t("extract")}
          </button>
          <button className="btn" onClick={() => onExtract([...selected], true)}>
            <Icon name="folderOut" size={18} />
            {t("extractTo")}
          </button>
          {editable && (selCount > 0 || focus) && (
            <>
              <button className="btn quiet" onClick={() => props.onDelete(targets(focus ?? undefined))}><Icon name="x" size={18} />{t("deleteBtn")}</button>
              {single && <button className="btn quiet" onClick={() => props.onRename(single)}>{t("renameBtn")}</button>}
            </>
          )}
          <span className="spacer" />
          <button className="btn quiet" onClick={onClose}>{t("close")}</button>
        </footer>
      </section>

      {drawer && <div className="drawer-veil narrow-only" onClick={() => setDrawer(false)} aria-hidden="true" />}
      <aside className="peek" aria-label={t("previewTitle")}>
        <div className="tabs-row">
          <div className="tabs" role="tablist">
            <button role="tab" aria-selected={tab === "preview"} className={tab === "preview" ? "on" : ""} onClick={() => setTab("preview")}>{t("previewTitle")}</button>
            {isZip && <button role="tab" aria-selected={tab === "history"} className={tab === "history" ? "on" : ""} onClick={() => setTab("history")}>{t("history")}</button>}
            <button role="tab" aria-selected={tab === "health"} className={tab === "health" ? "on" : ""} onClick={() => setTab("health")}>{t("repair")}</button>
          </div>
          <button className="btn quiet icon-btn narrow-only" onClick={() => setDrawer(false)} aria-label={t("close")} title={t("close")}><Icon name="x" size={18} /></button>
        </div>
        {tab === "history" && (
          <ol className="versions">
            {versions?.length === 0 && <li className="peek-msg">{t("historyEmpty")}</li>}
            {versions?.slice().reverse().map((v) => (
              <li key={v.version}>
                <b>{t("version", { v: num(v.version, lang) })}</b>
                <span className="path">{new Date(v.time * 1000).toLocaleString(lang === "bn" ? "bn-BD" : undefined, { dateStyle: "medium", timeStyle: "short" })}</span>
                <span className="vchips">
                  {v.added.length > 0 && <span className="vchip add">+{num(v.added.length, lang)}</span>}
                  {v.changed.length > 0 && <span className="vchip chg">~{num(v.changed.length, lang)}</span>}
                  {v.removed.length > 0 && <span className="vchip del">−{num(v.removed.length, lang)}</span>}
                </span>
              </li>
            ))}
          </ol>
        )}
        {tab === "health" && <HealthPanel ctx={ctx} archive={info.path} />}
        {tab === "preview" && (
          <>
            <div className="peek-head"><p>{t("previewSub")}</p></div>
            <div className={`peek-box ${peek.kind}`}>
              {peek.kind === "idle" && <p className="peek-msg">{t("peekPick")}</p>}
              {peek.kind === "loading" && <div className="shimmer" />}
              {peek.kind === "image" && <img src={peek.url} alt={focus?.label} />}
              {peek.kind === "pdf" && <object data={peek.url} type="application/pdf" aria-label={focus?.label} />}
              {peek.kind === "text" && <pre>{peek.text}</pre>}
              {peek.kind === "folder" && <p className="peek-msg"><span className="ficon t-folder big"><Icon name="folder" size={28} /></span>{t("folderSummary", { files: n(peek.files), size: human(peek.size, lang) })}</p>}
              {peek.kind === "none" && <p className="peek-msg">{t("peekNone")}</p>}
              {peek.kind === "error" && <p className="peek-msg">{peek.message}</p>}
            </div>
            {focus?.kind === "file" && (
              <dl className="meta">
                <div><dt>{t("name")}</dt><dd title={focus.name}>{focus.label}</dd></div>
                <div><dt>{t("size")}</dt><dd>{human(focus.entry.size, lang)}</dd></div>
                {focus.entry.packed != null && <div><dt>{t("packed")}</dt><dd>{focus.entry.packed >= focus.entry.size ? t("stored") : human(focus.entry.packed, lang)}</dd></div>}
              </dl>
            )}
            <p className="hint drag-hint">{t("dragOutHint")}</p>
          </>
        )}
      </aside>
    </div>
  );
}
