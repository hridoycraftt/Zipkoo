import { useEffect, useState } from "react";
import { appInfo, passwordAdd, passwordRemove, passwordsList, asUiError, type AppInfo, type SavedPassword, type ShellStatus } from "../api";
import type { Lang, makeT } from "../i18n";
import type { Settings, Theme } from "../settings";
import { Icon, type IconName } from "./Icon";

type T = ReturnType<typeof makeT>;

function Seg<V extends string>({ value, options, onChange, label }: { value: V; options: [V, string, IconName?][]; onChange: (v: V) => void; label: string }) {
  return (
    <div className="seg wrap" role="radiogroup" aria-label={label}>
      {options.map(([v, l, icon]) => (
        <button key={v} type="button" role="radio" aria-checked={value === v} className={value === v ? "on" : ""} onClick={() => onChange(v)}>
          {icon && <Icon name={icon} size={16} />}
          {l}
        </button>
      ))}
    </div>
  );
}

function Toggle({ id, checked, onChange, label, hint }: { id: string; checked: boolean; onChange: (v: boolean) => void; label: string; hint?: string }) {
  return (
    <label className="set-row toggle-row" htmlFor={id}>
      <span className="set-text">
        <b>{label}</b>
        {hint && <small>{hint}</small>}
      </span>
      <span className="switch">
        <input id={id} type="checkbox" role="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        <span className="track" aria-hidden="true" />
      </span>
    </label>
  );
}

export function SettingsView(p: {
  scanner: string | null;
  t: T;
  lang: Lang;
  setLang: (l: Lang) => void;
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
  shell: ShellStatus;
  toggleShell: () => void;
  shareServer: string;
  setShareServer: (s: string) => void;
  checkUpdates: () => Promise<string>;
}) {
  const { t, settings: s, update } = p;
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [server, setServer] = useState(p.shareServer);
  const [upd, setUpd] = useState("");
  const [pws, setPws] = useState<SavedPassword[]>([]);
  const [newLabel, setNewLabel] = useState("");
  const [newPw, setNewPw] = useState("");
  const [pwErr, setPwErr] = useState("");
  useEffect(() => {
    appInfo().then(setInfo).catch(() => {});
    passwordsList().then(setPws).catch(() => {});
  }, []);

  return (
    <div className="page settings">
      <header className="page-head"><div><h1>{t("settingsTitle")}</h1></div></header>

      <section className="set-sec" aria-labelledby="sec-look">
        <h2 id="sec-look">{t("secLook")}</h2>
        <div className="set-row">
          <span className="set-text"><b>{t("themeLabel")}</b></span>
          <Seg<Theme> label={t("themeLabel")} value={s.theme} onChange={(theme) => update({ theme })} options={[["system", t("themeSystem"), "computer"], ["light", t("themeLight"), "sun"], ["dark", t("themeDark"), "moon"]]} />
        </div>
        <div className="set-row">
          <span className="set-text"><b>{t("language")}</b></span>
          <Seg<Lang> label={t("language")} value={p.lang} onChange={p.setLang} options={[["en", "English"], ["bn", "বাংলা"]]} />
        </div>
      </section>

      <section className="set-sec" aria-labelledby="sec-new">
        <h2 id="sec-new">{t("secNew")}</h2>
        <div className="set-row">
          <span className="set-text"><b>{t("defaultFormatLabel")}</b></span>
          <Seg label={t("defaultFormatLabel")} value={s.defaultFormat} onChange={(defaultFormat) => update({ defaultFormat })} options={[["zip", "ZIP"], ["seven-z", "7Z"], ["tar-zst", "TAR.ZST"]]} />
        </div>
        <div className="set-row">
          <span className="set-text"><b>{t("defaultLevelLabel")}</b></span>
          <Seg label={t("defaultLevelLabel")} value={s.defaultLevel} onChange={(defaultLevel) => update({ defaultLevel })} options={[["fast", t("fast")], ["normal", t("normal")], ["max", t("max")]]} />
        </div>
        <Toggle id="set-smart" checked={s.smart} onChange={(smart) => update({ smart })} label={t("smart")} hint={t("smartHint")} />
      </section>

      <section className="set-sec" aria-labelledby="sec-extract">
        <h2 id="sec-extract">{t("secExtract")}</h2>
        <Toggle id="set-ask" checked={s.extractAsk} onChange={(extractAsk) => update({ extractAsk })} label={t("extractAskLabel")} hint={t("extractAskHint")} />
        <Toggle id="set-open" checked={s.openAfterExtract} onChange={(openAfterExtract) => update({ openAfterExtract })} label={t("openAfterLabel")} />
        <Toggle id="set-trash" checked={s.trashAfterExtract} onChange={(trashAfterExtract) => update({ trashAfterExtract })} label={t("trashAfterLabel")} hint={t("trashAfterHint")} />
        <Toggle id="set-skip" checked={s.skipPrograms} onChange={(skipPrograms) => update({ skipPrograms })} label={t("skipProgramsLabel")} />
        {p.scanner ? (
          <Toggle id="set-scan" checked={s.scanAfterExtract} onChange={(scanAfterExtract) => update({ scanAfterExtract })} label={t("scanLabel")} hint={t("scanHint", { engine: p.scanner })} />
        ) : (
          <div className="set-row"><span className="set-text"><b>{t("scanLabel")}</b><small>{t("scanNone")}</small></span></div>
        )}
      </section>

      {p.shell.supported && (
        <section className="set-sec" aria-labelledby="sec-int">
          <h2 id="sec-int">{t("secIntegration")}</h2>
          <Toggle id="set-shell" checked={p.shell.installed} onChange={() => p.toggleShell()} label={t("rightClick")} hint={p.shell.installed ? t("rightClickOn") : t("rightClickOff")} />
        </section>
      )}

      <section className="set-sec" aria-labelledby="sec-share">
        <h2 id="sec-share">{t("secSharing")}</h2>
        <label className="field">
          <span>{t("serverLabel")}</span>
          <div className="row gap nowrap">
            <input id="set-server" placeholder="https://share.yourbrand.com" value={server} onChange={(e) => setServer(e.target.value)} />
            <button className="btn" disabled={server.trim().replace(/\/$/, "") === p.shareServer} onClick={() => p.setShareServer(server.trim().replace(/\/$/, ""))}>{t("serverSave")}</button>
          </div>
          <small>{t("serverHint")}</small>
        </label>
      </section>

      <section className="set-sec" aria-labelledby="sec-pw">
        <h2 id="sec-pw">{t("secPasswords")}</h2>
        <p className="set-note">{t("rememberPwHint")}</p>
        {pws.length === 0 ? (
          <p className="set-note">{t("noSavedPw")}</p>
        ) : (
          <ul className="pw-list">
            {pws.map((x) => (
              <li key={x.id} className="set-row">
                <span className="set-text">
                  <b><Icon name="lock" size={15} /> {x.label}</b>
                  <small>{t("savedOn", { when: new Date(x.created * 1000).toLocaleDateString(p.lang === "bn" ? "bn-BD" : undefined, { dateStyle: "medium" }) })}</small>
                </span>
                <button className="btn quiet small" onClick={async () => setPws(await passwordRemove(x.id))}>{t("remove")}</button>
              </li>
            ))}
          </ul>
        )}
        <form className="pw-add" onSubmit={async (e) => {
          e.preventDefault();
          setPwErr("");
          try {
            setPws(await passwordAdd(newLabel.trim() || t("password"), newPw));
            setNewLabel("");
            setNewPw("");
          } catch (err) {
            setPwErr(asUiError(err).message);
          }
        }}>
          <input id="pw-label" placeholder={t("pwLabelPlaceholder")} aria-label={t("pwLabel")} value={newLabel} onChange={(e) => setNewLabel(e.target.value)} />
          <input id="pw-secret" type="password" autoComplete="new-password" placeholder={t("password")} aria-label={t("password")} value={newPw} onChange={(e) => setNewPw(e.target.value)} />
          <button className="btn small" type="submit" disabled={!newPw}>{t("addPw")}</button>
        </form>
        {pwErr && <p className="err small">{pwErr}</p>}
      </section>

      <section className="set-sec" aria-labelledby="sec-upd">
        <h2 id="sec-upd">{t("secUpdates")}</h2>
        <div className="set-row">
          <span className="set-text">
            <b>{info ? t("versionLabel", { v: info.version }) : "…"}</b>
            <small>{info?.flavor === "appstore" ? t("updatesStore") : info && !info.updater_ready ? t("updatesNotSetUp") : upd}</small>
          </span>
          {info?.flavor === "direct" && info.updater_ready && (
            <button className="btn small" onClick={async () => { setUpd(t("checking2")); setUpd(await p.checkUpdates()); }}>{t("checkNow")}</button>
          )}
        </div>
        {info?.flavor === "direct" && info.updater_ready && (
          <Toggle id="set-auto" checked={s.autoUpdate} onChange={(autoUpdate) => update({ autoUpdate })} label={t("autoUpdateLabel")} />
        )}
      </section>
    </div>
  );
}
