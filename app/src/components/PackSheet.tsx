import { useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { FORMAT_EXT, type Level, type PackRequest } from "../api";
import type { Lang, makeT } from "../i18n";
import { baseName, dirName, num, sep, toBn } from "../util";

type CreateFormat = PackRequest["format"];

interface Props {
  t: ReturnType<typeof makeT>;
  lang: Lang;
  inputs: string[];
  onCancel: () => void;
  onStart: (req: PackRequest) => void;
}

const SPLITS: { label: string; bytes: number | null }[] = [
  { label: "—", bytes: null },
  { label: "25 MB", bytes: 25 * 1024 ** 2 },
  { label: "700 MB", bytes: 700 * 1024 ** 2 },
  { label: "2 GB", bytes: 2 * 1024 ** 3 },
  { label: "4 GB", bytes: 4 * 1024 ** 3 - 1 },
];

function defaultOutput(inputs: string[], format: CreateFormat) {
  const first = inputs[0];
  const s = sep(first);
  const b = baseName(first);
  const name = inputs.length === 1 ? b.replace(/\.[^.]+$/, "") || b : "Archive";
  return `${dirName(first)}${s}${name || "Archive"}.${FORMAT_EXT[format]}`;
}

export function PackSheet({ t, lang, inputs, onCancel, onStart }: Props) {
  const [format, setFormat] = useState<CreateFormat>("zip");
  const [level, setLevel] = useState<Level>("normal");
  const [smart, setSmart] = useState(true);
  const [lock, setLock] = useState(false);
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [hideNames, setHideNames] = useState(true);
  const [split, setSplit] = useState(0);
  const [customOut, setCustomOut] = useState<string | null>(null);

  const output = useMemo(() => {
    if (!customOut) return defaultOutput(inputs, format);
    return customOut.replace(/\.(zip|7z|tar\.zst)$/i, "") + "." + FORMAT_EXT[format];
  }, [customOut, inputs, format]);

  const canLock = format !== "tar-zst";
  const pwError = lock && canLock ? (pw.length < 6 ? t("pwShort") : pw !== pw2 ? t("pwMismatch") : null) : null;
  const showPwError = lock && (pw.length > 0 || pw2.length > 0) && pwError;

  const pick = async () => {
    const p = await save({ defaultPath: output, filters: [{ name: "Archive", extensions: [FORMAT_EXT[format].split(".").pop()!] }] });
    if (p) setCustomOut(p);
  };

  const submit = (ev: React.FormEvent) => {
    ev.preventDefault();
    if (pwError) return;
    onStart({
      inputs,
      output,
      format,
      level,
      smart,
      password: lock && canLock ? pw : null,
      hideNames: format === "seven-z" && hideNames,
      splitSize: SPLITS[split].bytes,
    });
  };

  const hints: Record<CreateFormat, string> = { zip: t("zipHint"), "seven-z": t("szHint"), "tar-zst": t("tzHint") };
  const n = inputs.length;

  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="pack-title">
      <form className="sheet" onSubmit={submit}>
        <h2 id="pack-title">{t("packTitle", { n: lang === "bn" ? toBn(String(n)) : n })}</h2>
        <ul className="inputs mono small">
          {inputs.slice(0, 4).map((p) => <li key={p} title={p}>{baseName(p)}</li>)}
          {n > 4 && <li className="muted">+{num(n - 4, lang)}</li>}
        </ul>

        <fieldset>
          <legend>{t("format")}</legend>
          <div className="seg" role="radiogroup">
            {(["zip", "seven-z", "tar-zst"] as CreateFormat[]).map((f) => (
              <button type="button" key={f} role="radio" aria-checked={format === f} className={format === f ? "on" : ""} onClick={() => setFormat(f)}>
                {FORMAT_EXT[f].toUpperCase()}
              </button>
            ))}
          </div>
          <p className="hint">{hints[format]}</p>
        </fieldset>

        <fieldset>
          <legend>{t("level")}</legend>
          <div className="seg" role="radiogroup">
            {(["fast", "normal", "max"] as Level[]).map((l) => (
              <button type="button" key={l} role="radio" aria-checked={level === l} className={level === l ? "on" : ""} onClick={() => setLevel(l)}>
                {t(l)}
              </button>
            ))}
          </div>
        </fieldset>

        <label className="toggle">
          <input id="smart" type="checkbox" checked={smart} onChange={(e) => setSmart(e.target.checked)} />
          <span><b>{t("smart")}</b><small>{t("smartHint")}</small></span>
        </label>

        {canLock && (
          <label className="toggle">
            <input id="lock" type="checkbox" checked={lock} onChange={(e) => setLock(e.target.checked)} />
            <span><b>🔒 {t("lock")}</b><small>{t("lockHint")}</small></span>
          </label>
        )}
        {lock && canLock && (
          <div className="pw">
            <input id="pw1" type="password" autoComplete="new-password" placeholder={t("password")} value={pw} onChange={(e) => setPw(e.target.value)} />
            <input id="pw2" type="password" autoComplete="new-password" placeholder={t("repeatPassword")} value={pw2} onChange={(e) => setPw2(e.target.value)} />
            {showPwError && <p className="err small">{pwError}</p>}
            {format === "seven-z" && (
              <label className="check small">
                <input id="hide-names" type="checkbox" checked={hideNames} onChange={(e) => setHideNames(e.target.checked)} />
                {t("hideNames")}
              </label>
            )}
            {format === "zip" && <p className="hint">{t("lockZipNote")}</p>}
          </div>
        )}

        <fieldset>
          <legend>{t("split")}</legend>
          <div className="seg wrap" role="radiogroup">
            {SPLITS.map((s, i) => (
              <button type="button" key={s.label} role="radio" aria-checked={split === i} className={split === i ? "on" : ""} onClick={() => setSplit(i)}>
                {i === 0 ? t("noSplit") : s.label}
              </button>
            ))}
          </div>
        </fieldset>

        <div className="saveas">
          <span className="eyebrow">{t("saveAs")}</span>
          <span className="mono small path" title={output}>{output}</span>
          <button type="button" className="btn ghost small" onClick={pick}>{t("change")}</button>
        </div>

        <div className="row gap end">
          <button type="button" className="btn ghost" onClick={onCancel}>{t("cancel")}</button>
          <button type="submit" className="btn primary" disabled={!!pwError}>{t("zipIt")}</button>
        </div>
      </form>
    </div>
  );
}
