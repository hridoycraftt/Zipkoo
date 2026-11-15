import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  asUiError, convertArchives, findDuplicates, healCheck, healCreate, healStatus, indexRefresh, listArchives, onEvent, searchArchives,
  shareDelete, shareDownload, sharesList, watchRemove, watchRunNow, watchSave, watchesList,
  type CloudFolder, type DupGroup, type FolderItem, type Health, type Hit, type IndexStats, type ShareRecord, type Watch,
} from "../api";
import type { Lang, makeT } from "../i18n";
import { baseName, dirName, human, num } from "../util";
import { fileIcon, Icon } from "./Icon";
import type { ToastData } from "./Overlays";

type T = ReturnType<typeof makeT>;
export type RunJob = <R>(title: string, fn: (jobId: string) => Promise<R>) => Promise<R | undefined>;

export interface ViewCtx {
  t: T;
  lang: Lang;
  toast: (t: ToastData) => void;
  runJob: RunJob;
  openPath: (p: string) => void;
  fail: (e: unknown) => void;
}

const when = (secs: number, lang: Lang) => new Date(secs * 1000).toLocaleString(lang === "bn" ? "bn-BD" : undefined, { dateStyle: "medium", timeStyle: "short" });

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

function Page({ title, sub, children, aside }: { title: string; sub?: string; children: React.ReactNode; aside?: React.ReactNode }) {
  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>{title}</h1>
          {sub && <p className="lede">{sub}</p>}
        </div>
        {aside}
      </header>
      {children}
    </div>
  );
}

// ---------- Search ----------

export function SearchView({ ctx, extra }: { ctx: ViewCtx; extra: string[] }) {
  const { t, lang } = ctx;
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [stats, setStats] = useState<IndexStats | null>(null);
  const [busy, setBusy] = useState(true);
  const [folders, setFolders] = useState<string[]>([]);

  const refresh = useCallback(async (more: string[]) => {
    setBusy(true);
    try {
      setStats(await indexRefresh([...extra, ...more]));
    } catch (e) {
      ctx.fail(e);
    } finally {
      setBusy(false);
    }
  }, [extra, ctx]);

  useEffect(() => {
    refresh(folders);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!q.trim()) {
      setHits([]);
      return;
    }
    const id = setTimeout(() => searchArchives(q).then(setHits).catch(() => setHits([])), 160);
    return () => clearTimeout(id);
  }, [q, stats]);

  const addFolder = async () => {
    const d = await openDialog({ directory: true });
    if (!d || Array.isArray(d)) return;
    const next = [...folders, d];
    setFolders(next);
    refresh(next);
  };

  return (
    <Page
      title={t("searchTitle")}
      sub={t("searchSub")}
      aside={<button className="btn" onClick={addFolder}><Icon name="folder" />{t("addFolderToSearch")}</button>}
    >
      <label className="search big">
        <Icon name="search" size={20} />
        <input id="global-search" autoFocus placeholder={t("searchPlaceholder")} value={q} onChange={(e) => setQ(e.target.value)} />
      </label>
      <p className="meta-line">{busy ? t("indexing") : stats ? t("indexStats", { archives: num(stats.archives, lang), files: num(stats.files, lang) }) : ""}</p>
      <ul className="rows">
        {q.trim() && hits.length === 0 && !busy && <li className="empty">{t("noHits", { q })}</li>}
        {hits.map((h) => {
          const fi = fileIcon(h.name, false);
          return (
            <li key={h.archive + h.name}>
              <button className="row-btn" onClick={() => ctx.openPath(h.archive)} title={h.archive}>
                <span className={`ficon ${fi.tint}`}><Icon name={fi.icon} size={18} /></span>
                <span className="names">
                  <span className="leaf">{h.name.split("/").pop()}</span>
                  <span className="path">{t("inArchive", { name: baseName(h.archive) })} · {h.name}</span>
                </span>
                <span className="num">{human(h.size, lang)}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </Page>
  );
}

// ---------- Watch folders ----------

const INTERVALS = [15, 60, 360, 1440];

export function WatchView({ ctx }: { ctx: ViewCtx }) {
  const { t, lang } = ctx;
  const [list, setList] = useState<Watch[]>([]);
  const [running, setRunning] = useState<string | null>(null);

  const reload = useCallback(() => watchesList().then(setList).catch(ctx.fail), [ctx]);
  useEffect(() => {
    reload();
    const un = onEvent("watches-updated", reload);
    return () => {
      un.then((f) => f());
    };
  }, [reload]);

  const save = async (w: Watch) => {
    try {
      setList(await watchSave(w));
    } catch (e) {
      ctx.fail(e);
    }
  };

  const add = async () => {
    const folder = await openDialog({ directory: true });
    if (!folder || Array.isArray(folder)) return;
    const dest = await openDialog({ directory: true, title: t("pickDest"), defaultPath: dirName(folder) });
    if (!dest || Array.isArray(dest)) return;
    save({ id: "", folder, dest, intervalMinutes: 60, keep: 10, repair: false, paused: false, lastRun: null, lastResult: null, lastOk: true });
  };

  const label = (m: number) => (m < 60 ? `${num(m, lang)} ${t("minutes")}` : `${num(m / 60, lang)} ${t("hours")}`);

  return (
    <Page title={t("watchTitle")} sub={t("watchSub")} aside={<button className="btn primary" onClick={add}><Icon name="folder" />{t("addWatch")}</button>}>
      {list.length === 0 && <p className="empty pad">{t("watchEmpty")}</p>}
      <ul className="watch-list">
        {list.map((w) => (
          <li key={w.id} className={`watch ${w.paused ? "is-paused" : ""}`}>
            <div className="watch-main">
              <span className="ficon t-folder"><Icon name="folder" size={18} /></span>
              <div className="names">
                <span className="leaf">{baseName(w.folder)}{w.paused && <span className="flag muted-flag">{t("paused")}</span>}</span>
                <span className="path">{t("backupTo")}: {w.dest}</span>
                <span className={`path ${w.lastOk ? "" : "err"}`}>
                  {w.lastRun ? t("lastRun", { when: when(w.lastRun, lang), result: w.lastResult ?? "" }) : t("neverRun")}
                </span>
              </div>
            </div>
            <div className="watch-controls">
              <label className="inline-field">
                {t("every")}
                <select value={w.intervalMinutes} onChange={(e) => save({ ...w, intervalMinutes: Number(e.target.value) })}>
                  {INTERVALS.map((m) => <option key={m} value={m}>{label(m)}</option>)}
                </select>
              </label>
              <label className="inline-field">
                {t("keepN")}
                <select value={w.keep} onChange={(e) => save({ ...w, keep: Number(e.target.value) })}>
                  {[5, 10, 30, 0].map((k) => <option key={k} value={k}>{k === 0 ? "∞" : num(k, lang)}</option>)}
                </select>
                {t("backups")}
              </label>
              <label className="check small">
                <input type="checkbox" checked={w.repair} onChange={(e) => save({ ...w, repair: e.target.checked })} />
                {t("withRepair")}
              </label>
              <span className="watch-buttons">
              <button className="btn small" disabled={running === w.id} onClick={async () => { setRunning(w.id); setList(await watchRunNow(w.id).catch(() => list)); setRunning(null); }}>
                {running === w.id ? t("working") : t("runNow")}
              </button>
              <button className="btn quiet small" onClick={() => save({ ...w, paused: !w.paused })}>{w.paused ? t("resume") : t("pause")}</button>
              <button className="btn quiet small" onClick={async () => setList(await watchRemove(w.id))}>{t("remove")}</button>
              </span>
            </div>
          </li>
        ))}
      </ul>
    </Page>
  );
}

// ---------- Shared links ----------

export function SharesView({ ctx, server, setServer, askPassword }: { ctx: ViewCtx; server: string; setServer: (s: string) => void; askPassword: (name: string, retry: (pw: string) => void, wrong?: boolean) => void }) {
  const { t, lang } = ctx;
  const [list, setList] = useState<ShareRecord[]>([]);
  const [draft, setDraft] = useState(server);
  const [link, setLink] = useState("");
  const reload = useCallback(() => sharesList().then(setList).catch(ctx.fail), [ctx]);
  useEffect(() => {
    reload();
    const un = onEvent("shares-updated", reload);
    return () => {
      un.then((f) => f());
    };
  }, [reload]);

  const download = async () => {
    const dest = await openDialog({ directory: true });
    if (!dest || Array.isArray(dest)) return;
    const target = link.trim();
    // The password is checked before any bytes download, so retries don't use up the link.
    const run = async (pw?: string): Promise<void> => {
      const r = await ctx.runJob(t("downloadTo"), async (id) => {
        try {
          return { path: await shareDownload(id, target, dest, pw) };
        } catch (e) {
          const err = asUiError(e);
          if (err.code === "password_required" || err.code === "wrong_password") return { needPw: err.code === "wrong_password" };
          throw e;
        }
      });
      if (!r) return;
      const got = r.path;
      if (got) {
        ctx.toast({ kind: "ok", text: baseName(got), action: { label: t("showInFolder"), run: () => revealItemInDir(got) } });
        setLink("");
      } else {
        askPassword(t("openLink"), (p2) => run(p2), r.needPw);
      }
    };
    run();
  };

  const now = Date.now() / 1000;
  return (
    <Page title={t("sharesTitle")} sub={t("sharesSub")}>
      <section className="block">
        <label className="field">
          <span>{t("serverLabel")}</span>
          <div className="row gap nowrap">
            <input id="share-server" placeholder="https://share.yourbrand.com" value={draft} onChange={(e) => setDraft(e.target.value)} />
            <button className="btn" disabled={draft.trim() === server} onClick={() => { setServer(draft.trim().replace(/\/$/, "")); ctx.toast({ kind: "ok", text: t("serverSaved") }); }}>{t("serverSave")}</button>
          </div>
          <small>{t("serverHint")}</small>
        </label>
      </section>
      <section className="block">
        <label className="field">
          <span>{t("openLink")}</span>
          <div className="row gap nowrap">
            <input id="share-link" placeholder={t("openLinkPlaceholder")} value={link} onChange={(e) => setLink(e.target.value)} />
            <button className="btn primary" disabled={!link.includes("/s/") || !link.includes("#")} onClick={() => download()}><Icon name="extract" />{t("downloadTo")}</button>
          </div>
        </label>
      </section>
      <ul className="rows">
        {list.length === 0 && <li className="empty">{t("noShares")}</li>}
        {list.map((s) => {
          const gone = s.expiresAt <= now;
          return (
            <li key={s.id} className="share-row">
              <span className="ficon t-arch"><Icon name={s.password ? "lock" : "zip"} size={18} /></span>
              <span className="names">
                <span className="leaf">{s.name}</span>
                <span className={`path ${gone ? "err" : ""}`}>
                  {gone ? t("expired") : t("expires", { when: when(s.expiresAt, lang) })} · {human(s.size, lang)}
                  {s.maxDownloads ? ` · ${t("downloadsMax", { n: num(s.maxDownloads, lang) })}` : ""}
                </span>
              </span>
              {!gone && <button className="btn small" onClick={async () => ctx.toast({ kind: (await copy(s.url)) ? "ok" : "info", text: t("linkCopied"), sub: s.url })}>{t("copyLink")}</button>}
              <button className="btn quiet small" onClick={async () => { try { await shareDelete(s.id); reload(); } catch (e) { ctx.fail(e); } }}>{t("deleteLink")}</button>
            </li>
          );
        })}
      </ul>
    </Page>
  );
}

export function ShareSheet({ t, name, onCancel, onCreate }: { t: T; name: string; onCancel: () => void; onCreate: (hours: number, downloads: number | null, password: string | null) => void }) {
  const [hours, setHours] = useState(24);
  const [limit, setLimit] = useState<number | null>(null);
  const [lock, setLock] = useState(false);
  const [pw, setPw] = useState("");
  const H: [number, string][] = [[1, t("oneHour")], [24, t("oneDay")], [72, t("threeDays")], [168, t("oneWeek")]];
  const L: [number | null, string][] = [[null, t("noLimit")], [1, "1"], [5, "5"], [20, "20"]];
  return (
    <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="share-title">
      <form className="sheet narrow" onSubmit={(e) => { e.preventDefault(); onCreate(hours, limit, lock ? pw : null); }}>
        <div className="dialog-icon"><Icon name="lock" size={22} /></div>
        <h2 id="share-title">{t("shareTitle", { name })}</h2>
        <p>{t("shareSub")}</p>
        <fieldset>
          <legend>{t("expiresIn")}</legend>
          <div className="seg wrap">{H.map(([h, l]) => <button type="button" key={h} className={hours === h ? "on" : ""} aria-pressed={hours === h} onClick={() => setHours(h)}>{l}</button>)}</div>
        </fieldset>
        <fieldset>
          <legend>{t("downloadLimit")}</legend>
          <div className="seg wrap">{L.map(([n, l]) => <button type="button" key={l} className={limit === n ? "on" : ""} aria-pressed={limit === n} onClick={() => setLimit(n)}>{l}</button>)}</div>
        </fieldset>
        <label className="toggle">
          <input type="checkbox" checked={lock} onChange={(e) => setLock(e.target.checked)} />
          <span><b>{t("shareLock")}</b><small>{t("shareLockHint")}</small></span>
        </label>
        {lock && <input type="password" autoComplete="new-password" placeholder={t("password")} value={pw} onChange={(e) => setPw(e.target.value)} />}
        <div className="row gap end">
          <button type="button" className="btn quiet" onClick={onCancel}>{t("cancel")}</button>
          <button type="submit" className="btn primary" disabled={lock && pw.length < 6}>{t("createLink")}</button>
        </div>
      </form>
    </div>
  );
}

// ---------- Tools ----------

export function HealthPanel({ ctx, archive }: { ctx: ViewCtx; archive: string }) {
  const { t, lang } = ctx;
  const [has, setHas] = useState<boolean | null>(null);
  const [health, setHealth] = useState<Health | null>(null);
  const [msg, setMsg] = useState<string>("");
  const reload = useCallback(() => healStatus(archive).then((s) => setHas(s.has_repair_data)).catch(() => setHas(false)), [archive]);
  useEffect(() => {
    setHealth(null);
    setMsg("");
    reload();
  }, [reload]);

  const check = async (fix: boolean) => {
    const h = await ctx.runJob(t("checking"), (id) => healCheck(id, archive, fix));
    if (!h) return;
    setHealth(h);
    setMsg(
      h.outdated ? t("repairOutdated")
      : h.repaired > 0 ? t("damagedFixed", { n: num(h.repaired, lang) })
      : h.damaged === 0 && h.size_ok ? t("healthy", { blocks: num(h.blocks, lang) })
      : h.unrecoverable > 0 ? t("damagedLost", { n: num(h.unrecoverable, lang) })
      : t("damagedFixable", { n: num(h.damaged || 1, lang) }),
    );
  };
  const add = async () => {
    const r = await ctx.runJob(t("working"), (id) => healCreate(id, archive, 10));
    if (r) {
      setMsg(t("repairAdded", { size: human(r.size, lang) }));
      reload();
    }
  };
  const fixable = health && !health.outdated && (health.damaged > 0 || !health.size_ok) && health.unrecoverable === 0 && health.repaired === 0;
  return (
    <div className="health">
      {has === false && <p className="muted-p">{t("noRepairData")}</p>}
      {msg && <p className={`health-msg ${health && (health.unrecoverable > 0 || health.outdated) ? "err" : "ok-text"}`}>{msg}</p>}
      {health?.outdated && <button className="btn small" onClick={add}>{t("addRepair")}</button>}
      <div className="row gap">
        {has && <button className="btn small" onClick={() => check(false)}>{t("checkHealth")}</button>}
        {fixable && <button className="btn primary small" onClick={() => check(true)}>{t("repair")}</button>}
        {has === false && <button className="btn small" onClick={add}>{t("addRepair")}</button>}
      </div>
    </div>
  );
}

export function ToolsView({ ctx }: { ctx: ViewCtx }) {
  const { t, lang } = ctx;
  const [to, setTo] = useState<"zip" | "seven-z" | "tar-zst">("seven-z");
  const [results, setResults] = useState<{ source: string; output: string | null; error: string | null }[]>([]);
  const [dupes, setDupes] = useState<DupGroup[] | null>(null);
  const [repairTarget, setRepairTarget] = useState<string | null>(null);

  const convert = async () => {
    const res = await openDialog({ multiple: true, filters: [{ name: "Archives", extensions: ["zip", "7z", "rar", "tar", "gz", "tgz", "zst", "tzst", "bz2", "xz", "001"] }] });
    if (!res) return;
    const files = Array.isArray(res) ? res : [res];
    const r = await ctx.runJob(t("converting"), (id) => convertArchives(id, files, to));
    if (r) {
      setResults(r);
      const ok = r.filter((x) => x.output).length;
      ctx.toast({ kind: ok === r.length ? "ok" : "err", text: t("convertDone", { n: num(ok, lang), total: num(r.length, lang) }) });
    }
  };
  const dup = async () => {
    const d = await openDialog({ directory: true });
    if (!d || Array.isArray(d)) return;
    const r = await ctx.runJob(t("working"), () => findDuplicates([d]));
    if (r) setDupes(r);
  };
  const pickRepair = async () => {
    const f = await openDialog({ filters: [{ name: "Archives", extensions: ["zip", "7z", "rar", "tar", "gz", "zst", "bz2", "xz", "exe"] }] });
    if (f && !Array.isArray(f)) setRepairTarget(f);
  };
  const wasted = dupes?.reduce((a, g) => a + g.wasted, 0) ?? 0;
  const labels: Record<typeof to, string> = { zip: "ZIP", "seven-z": "7Z", "tar-zst": "TAR.ZST" };

  return (
    <Page title={t("toolsTitle")}>
      <section className="tool">
        <div className="tool-head">
          <span className="ficon t-arch"><Icon name="zip" size={18} /></span>
          <div><h2>{t("convertTitle")}</h2><p>{t("convertSub")}</p></div>
        </div>
        <div className="row gap">
          <span className="small muted-inline">{t("convertTo")}</span>
          <div className="seg">{(Object.keys(labels) as (typeof to)[]).map((f) => <button key={f} className={to === f ? "on" : ""} aria-pressed={to === f} onClick={() => setTo(f)}>{labels[f]}</button>)}</div>
          <button className="btn primary" onClick={convert}>{t("convertPick")}</button>
        </div>
        {results.length > 0 && (
          <ul className="rows compact">
            {results.map((r) => (
              <li key={r.source} className="result-row">
                <Icon name={r.output ? "check" : "alert"} size={16} className={r.output ? "ok-text" : "err"} />
                <span className="ellipsis">{baseName(r.source)} → {r.output ? baseName(r.output) : r.error}</span>
                {r.output && <button className="link" onClick={() => revealItemInDir(r.output!)}>{t("showInFolder")}</button>}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="tool">
        <div className="tool-head">
          <span className="ficon t-image"><Icon name="image" size={18} /></span>
          <div><h2>{t("dupesTitle")}</h2><p>{t("dupesSub")}</p></div>
        </div>
        <div className="row gap"><button className="btn" onClick={dup}><Icon name="folder" />{t("dupesPick")}</button></div>
        {dupes && (
          <>
            <p className="meta-line">{dupes.length === 0 ? t("dupesNone") : t("dupesFound", { groups: num(dupes.length, lang), wasted: human(wasted, lang) })}</p>
            <ul className="rows compact">
              {dupes.slice(0, 50).map((g, i) => (
                <li key={i} className="dup">
                  <span className="dup-size">{num(g.files.length, lang)} × {human(g.size, lang)}</span>
                  <ul>{g.files.map((f) => <li key={f}><button className="link path-link" onClick={() => revealItemInDir(f)}>{f}</button></li>)}</ul>
                </li>
              ))}
            </ul>
          </>
        )}
      </section>

      <section className="tool">
        <div className="tool-head">
          <span className="ficon t-doc"><Icon name="shield" size={18} /></span>
          <div><h2>{t("repairTitle")}</h2><p>{t("repairSub")}</p></div>
        </div>
        <div className="row gap">
          <button className="btn" onClick={pickRepair}>{t("repairPick")}</button>
          {repairTarget && <span className="small ellipsis">{baseName(repairTarget)}</span>}
        </div>
        {repairTarget && <HealthPanel ctx={ctx} archive={repairTarget} />}
      </section>
    </Page>
  );
}

// ---------- Cloud folder ----------

export function FolderView({ ctx, folder }: { ctx: ViewCtx; folder: CloudFolder }) {
  const { t, lang } = ctx;
  const [items, setItems] = useState<FolderItem[] | null>(null);
  const ref = useRef(folder.path);
  useEffect(() => {
    ref.current = folder.path;
    setItems(null);
    listArchives(folder.path).then((v) => ref.current === folder.path && setItems(v));
  }, [folder.path]);
  return (
    <Page title={folder.label} sub={t("cloudSub", { provider: folder.label.split(" (")[0] })} aside={<button className="btn" onClick={() => revealItemInDir(folder.path)}><Icon name="folder" />{t("showInFolder")}</button>}>
      {items === null && <div className="shimmer" />}
      {items?.length === 0 && <p className="empty pad">{t("noArchivesHere")}</p>}
      <ul className="rows">
        {items?.map((it) => (
          <li key={it.path}>
            <button className="row-btn" onClick={() => ctx.openPath(it.path)} title={it.path}>
              <span className="ficon t-arch"><Icon name="zip" size={18} /></span>
              <span className="names">
                <span className="leaf">{baseName(it.path)}</span>
                <span className="path">{it.path.slice(folder.path.length + 1)} · {when(it.modified, lang)}</span>
              </span>
              <span className="num">{human(it.size, lang)}</span>
            </button>
          </li>
        ))}
      </ul>
    </Page>
  );
}
