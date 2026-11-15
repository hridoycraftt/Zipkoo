import { useEffect, useMemo, useState } from "react";
import { asUiError, FORMAT_LABEL, peekFile, type ArchiveInfo, type Entry } from "../api";
import type { Lang, makeT } from "../i18n";
import { baseName, human, mimeFor, num, previewKind } from "../util";

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

  return (
    <div className="archive">
      <div className="main-col">
        <header className="ahead">
          <div className="minw0">
            <h1 className="title" title={info.path}>{baseName(info.path)}</h1>
            <div className="mono muted small">
              {human(info.total_size, lang)} → {human(info.archive_size, lang)} ·{" "}
              <b className="ok">{t("savedPct", { pct: lang === "bn" ? num(saved, lang) : saved })}</b> · {num(info.file_count, lang)} {t("files")}
            </div>
          </div>
          <div className="chips">
            <span className="chip ink">{FORMAT_LABEL[info.format]}</span>
            {info.encrypted && <span className="chip teal">🔒 {t("encrypted")}</span>}
            <span className="chip teal">✓ {t("healthy")}</span>
          </div>
        </header>

        {progs > 0 && (
          <div className="banner warn" role="alert">
            <span>⚠ {t(progs === 1 ? "programsWarn" : "programsWarnMany", { n: num(progs, lang) })}</span>
            <label className="check">
              <input id="skip-programs" type="checkbox" checked={skipPrograms} onChange={(e) => setSkipPrograms(e.target.checked)} />
              {t("skipPrograms")}
            </label>
          </div>
        )}

        <input id="archive-search" className="search" placeholder={t("search")} value={query} onChange={(e) => setQuery(e.target.value)} />

        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th className="cb" aria-label="select" />
                <th>{t("name")}</th>
                <th>{t("type")}</th>
                <th className="num">{t("size")}</th>
                <th className="num">{t("packed")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((e) => (
                <tr
                  key={e.name}
                  className={focus?.name === e.name ? "focus" : ""}
                  onClick={() => setFocus(e)}
                  onDoubleClick={() => !e.is_dir && onExtract([e.name], false)}
                >
                  <td className="cb">
                    <input type="checkbox" aria-label={e.name} checked={selected.has(e.name)} onChange={() => toggle(e.name)} onClick={(ev) => ev.stopPropagation()} />
                  </td>
                  <td className="fname">
                    <span className={`ft ${e.is_dir ? "dir" : previewKind(e.name)}`} />
                    {e.name}
                    {e.executable && <span className="chip red tiny">program</span>}
                  </td>
                  <td className="muted">{e.is_dir ? t("folder") : (e.name.split(".").pop() ?? "").toUpperCase() || t("file")}</td>
                  <td className="num">{e.is_dir ? "" : human(e.size, lang)}</td>
                  <td className="num muted">
                    {e.is_dir || e.packed == null ? "" : e.packed >= e.size && e.size > 0 ? t("stored") : human(e.packed, lang)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <footer className="actions">
          <button className="btn primary" onClick={() => onExtract([...selected], false)}>
            {selected.size ? `${t("extractSelected")} (${num(selected.size, lang)})` : t("extract")}
          </button>
          <button className="btn" onClick={() => onExtract([...selected], true)}>{t("extractTo")}</button>
          <span className="spacer" />
          <button className="btn ghost" onClick={onClose}>{t("close")}</button>
        </footer>
      </div>

      <aside className="peek">
        <h2 className="eyebrow">{t("peek")}</h2>
        <div className="peek-box">
          {peek.kind === "idle" && <p className="muted small">{t("peekPick")}</p>}
          {peek.kind === "loading" && <div className="shimmer" />}
          {peek.kind === "image" && <img src={peek.url} alt={focus?.name} />}
          {peek.kind === "pdf" && <object data={peek.url} type="application/pdf" aria-label={focus?.name} />}
          {peek.kind === "text" && <pre>{peek.text}</pre>}
          {peek.kind === "none" && <p className="muted small">{t("peekNone")}</p>}
          {peek.kind === "error" && <p className="muted small">{peek.message}</p>}
        </div>
        {focus && !focus.is_dir && (
          <dl className="meta">
            <dt>{t("name")}</dt>
            <dd title={focus.name}>{baseName(focus.name)}</dd>
            <dt>{t("size")}</dt>
            <dd>{human(focus.size, lang)}</dd>
            {focus.packed != null && (
              <>
                <dt>{t("packed")}</dt>
                <dd>{focus.packed >= focus.size ? t("stored") : human(focus.packed, lang)}</dd>
              </>
            )}
          </dl>
        )}
      </aside>
    </div>
  );
}
