import { useEffect, useMemo, useState } from "react";
import { asUiError, FORMAT_LABEL, peekFile, type ArchiveInfo, type Entry } from "../api";
import type { Lang, makeT } from "../i18n";
import { baseName, human, mimeFor, num, previewKind } from "../util";
import { fileIcon, Icon } from "./Icon";

interface Props {
  t: ReturnType<typeof makeT>;
  lang: Lang;
  info: ArchiveInfo;
  password?: string;
  skipPrograms: boolean;
  setSkipPrograms: (v: boolean) => void;
  onExtract: (only: string[], chooseDest: boolean) => void;
  onClose: () => void;
}

type PeekState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "image"; url: string }
  | { kind: "pdf"; url: string }
  | { kind: "text"; text: string }
  | { kind: "none" }
  | { kind: "error"; message: string };

export function ArchiveView({ t, lang, info, password, skipPrograms, setSkipPrograms, onExtract, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<Entry | null>(null);
  const [peek, setPeek] = useState<PeekState>({ kind: "idle" });

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return info.entries.filter((e) => !q || e.name.toLowerCase().includes(q));
  }, [info.entries, query]);

  const saved = info.total_size > 0 ? Math.max(0, Math.round(100 - (info.archive_size / info.total_size) * 100)) : 0;

  useEffect(() => {
    let url: string | null = null;
    let live = true;
    if (!focus || focus.is_dir) {
      setPeek({ kind: "idle" });
      return;
    }
    const kind = previewKind(focus.name);
    if (kind === "none") {
      setPeek({ kind: "none" });
      return;
    }
    if (focus.size > 64 * 1024 * 1024) {
      setPeek({ kind: "error", message: t("peekTooBig") });
      return;
    }
    setPeek({ kind: "loading" });
    peekFile(info.path, focus.name, password)
      .then((bytes) => {
        if (!live) return;
        if (kind === "text") {
          setPeek({ kind: "text", text: new TextDecoder().decode(bytes.slice(0, 64 * 1024)) });
        } else {
          url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mimeFor(focus.name) }));
          setPeek({ kind, url });
        }
      })
      .catch((e) => live && setPeek({ kind: "error", message: asUiError(e).message }));
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

  const progs = info.executables.length;
  const n = (v: number) => num(v, lang);

  return (
    <div className="archive">
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
            {info.encrypted && (
              <li className="fact trust"><Icon name="lock" size={16} />{t("lockedNote")}</li>
            )}
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
          {selected.size > 0 && (
            <span className="selcount">
              {t("selectedN", { n: n(selected.size) })}
              <button className="link" onClick={() => setSelected(new Set())}>{t("clearSel")}</button>
            </span>
          )}
        </div>

        <div className="list" role="table" aria-label={baseName(info.path)}>
          <div className="list-head" role="row">
            <span role="columnheader" className="c-check" />
            <span role="columnheader">{t("name")}</span>
            <span role="columnheader" className="c-size">{t("size")}</span>
            <span role="columnheader" className="c-size">{t("packed")}</span>
          </div>
          <div className="list-body">
            {rows.length === 0 && <p className="empty">{t("noMatch", { q: query })}</p>}
            {rows.map((e) => {
              const fi = fileIcon(e.name, e.is_dir);
              const parts = e.name.replace(/\/$/, "").split("/");
              const leaf = parts.pop();
              const path = parts.join("/");
              return (
                <div
                  key={e.name}
                  role="row"
                  tabIndex={0}
                  aria-selected={focus?.name === e.name}
                  className={`row-item ${focus?.name === e.name ? "focus" : ""} ${selected.has(e.name) ? "picked" : ""}`}
                  onClick={() => setFocus(e)}
                  onKeyDown={(ev) => {
                    if (ev.key === " ") {
                      ev.preventDefault();
                      toggle(e.name);
                    }
                    if (ev.key === "Enter") setFocus(e);
                  }}
                  onDoubleClick={() => !e.is_dir && onExtract([e.name], false)}
                >
                  <span className="c-check" role="cell">
                    <input type="checkbox" aria-label={e.name} checked={selected.has(e.name)} onChange={() => toggle(e.name)} onClick={(ev) => ev.stopPropagation()} />
                  </span>
                  <span className="c-name" role="cell">
                    <span className={`ficon ${fi.tint}`}><Icon name={fi.icon} size={18} /></span>
                    <span className="names">
                      <span className="leaf">{leaf}</span>
                      {path && <span className="path">{path}</span>}
                    </span>
                    {e.executable && <span className="flag">{lang === "bn" ? "প্রোগ্রাম" : "Program"}</span>}
                  </span>
                  <span className="c-size" role="cell">{e.is_dir ? "" : human(e.size, lang)}</span>
                  <span className="c-size dim" role="cell">
                    {e.is_dir || e.packed == null ? "" : e.packed >= e.size && e.size > 0 ? t("stored") : human(e.packed, lang)}
                  </span>
                </div>
              );
            })}
          </div>
        </div>

        <footer className="actions">
          <button className="btn primary" onClick={() => onExtract([...selected], false)}>
            <Icon name="extract" size={18} />
            {selected.size ? `${t("extractSelected")} (${n(selected.size)})` : t("extract")}
          </button>
          <button className="btn" onClick={() => onExtract([...selected], true)}>
            <Icon name="folderOut" size={18} />
            {t("extractTo")}
          </button>
          <span className="spacer" />
          <button className="btn quiet" onClick={onClose}>{t("close")}</button>
        </footer>
      </section>

      <aside className="peek" aria-label={t("previewTitle")}>
        <div className="peek-head">
          <h2>{t("previewTitle")}</h2>
          <p>{t("previewSub")}</p>
        </div>
        <div className={`peek-box ${peek.kind}`}>
          {peek.kind === "idle" && <p className="peek-msg">{t("peekPick")}</p>}
          {peek.kind === "loading" && <div className="shimmer" />}
          {peek.kind === "image" && <img src={peek.url} alt={focus?.name} />}
          {peek.kind === "pdf" && <object data={peek.url} type="application/pdf" aria-label={focus?.name} />}
          {peek.kind === "text" && <pre>{peek.text}</pre>}
          {peek.kind === "none" && <p className="peek-msg">{t("peekNone")}</p>}
          {peek.kind === "error" && <p className="peek-msg">{peek.message}</p>}
        </div>
        {focus && !focus.is_dir && (
          <dl className="meta">
            <div><dt>{t("name")}</dt><dd title={focus.name}>{baseName(focus.name)}</dd></div>
            <div><dt>{t("size")}</dt><dd>{human(focus.size, lang)}</dd></div>
            {focus.packed != null && (
              <div><dt>{t("packed")}</dt><dd>{focus.packed >= focus.size ? t("stored") : human(focus.packed, lang)}</dd></div>
            )}
          </dl>
        )}
      </aside>
    </div>
  );
}
