import { useEffect, useRef, useState } from "react";
import type { Progress } from "../api";
import type { Lang, makeT } from "../i18n";
import { human, toBn } from "../util";
import { Icon } from "./Icon";

type T = ReturnType<typeof makeT>;

/** Progress closes like a zipper. */
export function ProgressOverlay({ t, lang, title, progress, onCancel }: { t: T; lang: Lang; title: string; progress: Progress | null; onCancel: () => void }) {
  const pct = progress && progress.total_bytes > 0 ? Math.min(100, (progress.done_bytes / progress.total_bytes) * 100) : 0;
  const pctLabel = `${pct.toFixed(0)}%`;
  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-live="polite">
      <div className="sheet narrow">
        <h2>{title}</h2>
        <div className="zipbar" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${pct}%` }} />
          <i style={{ left: `${pct}%` }} />
        </div>
        <div className="row between mono small muted">
          <span className="ellipsis">{progress?.current ?? ""}</span>
          <span>
            {progress ? `${human(progress.done_bytes, lang)} / ${human(progress.total_bytes, lang)} · ` : ""}
            {lang === "bn" ? toBn(pctLabel) : pctLabel}
          </span>
        </div>
        <div className="row end">
          <button className="btn quiet" onClick={onCancel}>{t("cancel")}</button>
        </div>
      </div>
    </div>
  );
}

export function PasswordDialog({ t, name, error, onSubmit, onCancel }: { t: T; name: string; error?: string; onSubmit: (pw: string) => void; onCancel: () => void }) {
  const [pw, setPw] = useState("");
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), [error]);
  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="pw-title">
      <form
        className="sheet narrow"
        onSubmit={(e) => {
          e.preventDefault();
          if (pw) onSubmit(pw);
        }}
      >
        <div className="dialog-icon"><Icon name="lock" size={22} /></div>
        <h2 id="pw-title">{t("pwTitle")}</h2>
        <p className="muted">{t("pwSub", { name })}</p>
        <input id="open-pw" ref={ref} type="password" autoComplete="current-password" placeholder={t("password")} value={pw} onChange={(e) => setPw(e.target.value)} />
        {error && <p className="err small">{error}</p>}
        <div className="row gap end">
          <button type="button" className="btn quiet" onClick={onCancel}>{t("cancel")}</button>
          <button type="submit" className="btn primary" disabled={!pw}>{t("open")}</button>
        </div>
      </form>
    </div>
  );
}

export interface ToastData {
  kind: "ok" | "err" | "info";
  text: string;
  sub?: string;
  action?: { label: string; run: () => void };
}

export function Toast({ toast, onDone }: { toast: ToastData; onDone: () => void }) {
  useEffect(() => {
    const id = setTimeout(onDone, toast.kind === "err" ? 9000 : 6000);
    return () => clearTimeout(id);
  }, [toast, onDone]);
  return (
    <div className={`toast ${toast.kind}`} role="status">
      <span className="toast-icon"><Icon name={toast.kind === "ok" ? "check" : toast.kind === "err" ? "alert" : "info"} size={18} /></span>
      <div>
        <b>{toast.text}</b>
        {toast.sub && <small>{toast.sub}</small>}
      </div>
      {toast.action && <button className="btn small" onClick={toast.action.run}>{toast.action.label}</button>}
      <button className="x" aria-label="Dismiss" onClick={onDone}><Icon name="x" size={16} /></button>
    </div>
  );
}
