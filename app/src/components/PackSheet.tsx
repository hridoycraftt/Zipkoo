import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { FORMAT_EXT, suggestPackOutput, type CloudFolder, type Level, type PackRequest } from "../api";
import type { Lang, makeT } from "../i18n";
import { baseName, num, toBn } from "../util";

/** What the user picks; "sfx" is a ZIP payload inside a self-extracting .exe. */
type Choice = PackRequest["format"] | "sfx";

const EXT: Record<Choice, string> = { ...FORMAT_EXT, sfx: "exe" };

export interface PackPreset {
  lock?: boolean;
  sfx?: boolean;
}

interface Props {
  t: ReturnType<typeof makeT>;
  lang: Lang;
  inputs: string[];
  sfxEnabled: boolean;
  clouds?: CloudFolder[];
  preset?: PackPreset;
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

export function PackSheet({ t, lang, inputs, sfxEnabled, clouds = [], preset, onCancel, onStart }: Props) {
  const [shrink, setShrink] = useState(false);
  const [repair, setRepair] = useState(false);
  const [choice, setChoice] = useState<Choice>(preset?.sfx && sfxEnabled ? "sfx" : "zip");
  const [level, setLevel] = useState<Level>("normal");
  const [smart, setSmart] = useState(true);
  const [lock, setLock] = useState(!!preset?.lock);
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [hideNames, setHideNames] = useState(true);
  const [split, setSplit] = useState(0);
  const [customOut, setCustomOut] = useState<string | null>(null);
  const [suggested, setSuggested] = useState<string>("");

  useEffect(() => {
    let live = true;
    suggestPackOutput(inputs, EXT[choice])
      .then((p) => live && p && setSuggested(p))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [inputs, choice]);

  const output = customOut ? customOut.replace(/\.(zip|7z|tar\.zst|exe)$/i, "") + "." + EXT[choice] : suggested;

  const canLock = choice !== "tar-zst";
  const canSplit = choice !== "sfx";
  const pwError = lock && canLock ? (pw.length < 6 ? t("pwShort") : pw !== pw2 ? t("pwMismatch") : null) : null;
  const showPwError = lock && (pw.length > 0 || pw2.length > 0) && pwError;

  const pick = async () => {
    const p = await save({ defaultPath: output, filters: [{ name: "Archive", extensions: [EXT[choice].split(".").pop()!] }] });
    if (p) setCustomOut(p);
  };

  const submit = (ev: React.FormEvent) => {
    ev.preventDefault();
    if (pwError || !output) return;
    const sfx = choice === "sfx";
    onStart({
      inputs,
      output,
      format: sfx ? "zip" : choice,
      level,
      smart,
      password: lock && canLock ? pw : null,
      hideNames: choice === "seven-z" && hideNames,
      splitSize: canSplit ? SPLITS[split].bytes : null,
      sfx,
      shrinkPhotos: shrink,
      repairPercent: repair && !sfx ? 10 : null,
    });
  };

  const choices: Choice[] = sfxEnabled ? ["zip", "seven-z", "tar-zst", "sfx"] : ["zip", "seven-z", "tar-zst"];
  const hints: Record<Choice, string> = { zip: t("zipHint"), "seven-z": t("szHint"), "tar-zst": t("tzHint"), sfx: t("sfxHint") };
  const n = inputs.length;

  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="pack-title">
      <form className="sheet" onSubmit={submit}>
        <h2 id="pack-title">{n === 1 ? t("packTitleOne") : t("packTitle", { n: lang === "bn" ? toBn(String(n)) : n })}</h2>
        <ul className="inputs">
          {inputs.slice(0, 4).map((p) => <li key={p} title={p}>{baseName(p)}</li>)}
          {n > 4 && <li className="muted">+{num(n - 4, lang)}</li>}
        </ul>

        <fieldset>
          <legend>{t("format")}</legend>
          <div className="seg" role="radiogroup">
            {choices.map((f) => (
              <button type="button" key={f} role="radio" aria-checked={choice === f} className={choice === f ? "on" : ""} onClick={() => setChoice(f)}>
                {f === "sfx" ? t("sfxFormat") : FORMAT_EXT[f].toUpperCase()}
              </button>
            ))}
          </div>
          <p className="hint">{hints[choice]}</p>
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

        <label className="toggle">
          <input id="shrink" type="checkbox" checked={shrink} onChange={(e) => setShrink(e.target.checked)} />
          <span><b>{t("shrinkPhotos")}</b><small>{t("shrinkHint")}</small></span>
        </label>

        {choice !== "sfx" && (
          <label className="toggle">
            <input id="repair" type="checkbox" checked={repair} onChange={(e) => setRepair(e.target.checked)} />
            <span><b>{t("repairData")}</b><small>{t("repairDataHint")}</small></span>
          </label>
        )}

        {canLock && (
          <label className="toggle">
            <input id="lock" type="checkbox" checked={lock} onChange={(e) => setLock(e.target.checked)} />
            <span><b>{t("lock")}</b><small>{t("lockHint")}</small></span>
          </label>
        )}
        {lock && canLock && (
          <div className="pw">
            <input id="pw1" type="password" autoComplete="new-password" placeholder={t("password")} value={pw} onChange={(e) => setPw(e.target.value)} />
            <input id="pw2" type="password" autoComplete="new-password" placeholder={t("repeatPassword")} value={pw2} onChange={(e) => setPw2(e.target.value)} />
            {showPwError && <p className="err small">{pwError}</p>}
            {choice === "seven-z" && (
              <label className="check small">
                <input id="hide-names" type="checkbox" checked={hideNames} onChange={(e) => setHideNames(e.target.checked)} />
                {t("hideNames")}
              </label>
            )}
            {choice === "zip" && <p className="hint">{t("lockZipNote")}</p>}
          </div>
        )}

        {canSplit && (
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
        )}

        {clouds.length > 0 && (
          <div className="cloud-chips">
            <span className="saveas-label">{t("cloudSave")}</span>
            {clouds.map((c) => (
              <button type="button" key={c.path} className="chip-btn" onClick={() => setCustomOut(`${c.path}${c.path.includes("\\") ? "\\" : "/"}${(output.split(/[\\/]/).pop() ?? "Archive").replace(/\.(zip|7z|tar\.zst|exe)$/i, "")}`)}>
                {c.label}
              </button>
            ))}
          </div>
        )}

        <div className="saveas">
          <span className="saveas-label">{t("saveAs")}</span>
          <span className="mono small path" title={output}><bdi>{output}</bdi></span>
          <button type="button" className="btn quiet small" onClick={pick}>{t("change")}</button>
        </div>

        <div className="row gap end">
          <button type="button" className="btn quiet" onClick={onCancel}>{t("cancel")}</button>
          <button type="submit" className="btn primary" disabled={!!pwError || !output}>{t("zipIt")}</button>
        </div>
      </form>
    </div>
  );
}
