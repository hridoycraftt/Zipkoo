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

export function PasswordDialog({ t, name, error, onSubmit, onCancel, canRemember = true }: { t: T; name: string; error?: string; onSubmit: (pw: string, remember: boolean) => void; onCancel: () => void; canRemember?: boolean }) {
  const [pw, setPw] = useState("");
  const [remember, setRemember] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), [error]);
  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="pw-title">
      <form
        className="sheet narrow"
        onSubmit={(e) => {
          e.preventDefault();
          if (pw) onSubmit(pw, remember);
        }}
      >
        <div className="dialog-icon"><Icon name="lock" size={22} /></div>
        <h2 id="pw-title">{t("pwTitle")}</h2>
        <p className="muted">{t("pwSub", { name })}</p>
        <input id="open-pw" ref={ref} type="password" autoComplete="current-password" placeholder={t("password")} value={pw} onChange={(e) => setPw(e.target.value)} />
        {error && <p className="err small">{error}</p>}
        {canRemember && (
          <label className="check small" title={t("rememberPwHint")}>
            <input id="remember-pw" type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
            {t("rememberPw")}
          </label>
        )}
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

/** A short yes/no question. `danger` styles the confirm button for destructive actions. */
export function ConfirmDialog({ title, body, confirm, cancel, danger, onConfirm, onCancel, children }: {
  title: string; body?: string; confirm: string; cancel: string; danger?: boolean;
  onConfirm: () => void; onCancel: () => void; children?: React.ReactNode;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => ref.current?.focus(), []);
  return (
    <div className="scrim" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" onKeyDown={(e) => e.key === "Escape" && onCancel()}>
      <div className="sheet narrow">
        <h2 id="confirm-title">{title}</h2>
        {body && <p>{body}</p>}
        {children}
        <div className="row gap end">
          <button className="btn quiet" onClick={onCancel}>{cancel}</button>
          <button ref={ref} className={`btn ${danger ? "danger" : "primary"}`} onClick={onConfirm}>{confirm}</button>
        </div>
      </div>
    </div>
  );
}

/** Ask for one line of text, e.g. a new name. */
export function PromptDialog({ title, label, initial, submit, cancel, validate, onSubmit, onCancel }: {
  title: string; label: string; initial: string; submit: string; cancel: string;
  validate: (v: string) => string | null; onSubmit: (v: string) => void; onCancel: () => void;
}) {
  const [v, setV] = useState(initial);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    // Select the name without its extension, like Finder and Explorer do.
    const dot = initial.lastIndexOf(".");
    el.setSelectionRange(0, dot > 0 ? dot : initial.length);
  }, [initial]);
  const err = v.trim() === initial ? null : validate(v.trim());
  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="prompt-title" onKeyDown={(e) => e.key === "Escape" && onCancel()}>
      <form className="sheet narrow" onSubmit={(e) => { e.preventDefault(); if (!err && v.trim() && v.trim() !== initial) onSubmit(v.trim()); }}>
        <h2 id="prompt-title">{title}</h2>
        <label className="field">
          <span>{label}</span>
          <input id="prompt-input" ref={ref} value={v} onChange={(e) => setV(e.target.value)} spellCheck={false} autoComplete="off" />
        </label>
        {err && <p className="err small">{err}</p>}
        <div className="row gap end">
          <button type="button" className="btn quiet" onClick={onCancel}>{cancel}</button>
          <button type="submit" className="btn primary" disabled={!!err || !v.trim() || v.trim() === initial}>{submit}</button>
        </div>
      </form>
    </div>
  );
}
