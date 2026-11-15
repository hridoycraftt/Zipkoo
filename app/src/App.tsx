import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath as osOpen, revealItemInDir } from "@tauri-apps/plugin-opener";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { check as checkUpdate } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { applyTheme, loadSettings, saveSettings, type Settings } from "./settings";
import { SettingsView } from "./components/SettingsView";
import {
  EDITABLE, appInfo, editArchive, extractConflicts, trashPath, openEntry, prepareDrag, type Change, type Entry,
  cloudFolders, convertArchives, shareFile, updateArchive, type CloudFolder,
  asUiError, cancelJob, extractArchive, inspectArchive, isArchive, onProgress, onShellActions, packFiles, sfxAvailable,
  shellSet, shellStatus, suggestExtractDest, suggestPackOutput, takeShellActions,
  type ArchiveInfo, type PackRequest, type Progress, type ShellAction, type ShellStatus, type UiError,
} from "./api";
import { makeT, type Lang } from "./i18n";
import { baseName, dirName, human, num, sep, stripArchiveExt, uid } from "./util";
import { Mark } from "./components/Mark";
import { Icon } from "./components/Icon";
import { Home } from "./components/Home";
import { ArchiveView } from "./components/ArchiveView";
import { PackSheet, type PackPreset } from "./components/PackSheet";
import { ConfirmDialog, PasswordDialog, ProgressOverlay, PromptDialog, Toast, type ToastData } from "./components/Overlays";
import { FolderView, SearchView, SharesView, ShareSheet, ToolsView, WatchView, type ViewCtx } from "./components/Views";

type View = { kind: "home" | "search" | "watch" | "shares" | "tools" | "settings" } | { kind: "cloud"; folder: CloudFolder };

const store = {
  get<T>(k: string, d: T): T {
    try {
      const v = localStorage.getItem(k);
      return v ? (JSON.parse(v) as T) : d;
    } catch {
      return d;
    }
  },
  set(k: string, v: unknown) {
    try {
      localStorage.setItem(k, JSON.stringify(v));
    } catch {
      /* storage unavailable */
    }
  },
};

type Job = { id: string; kind: "pack" | "extract"; title?: string; progress: Progress | null };
type PwPrompt = { path: string; error?: string; then: (pw: string) => void; cancel?: () => void };

export default function App() {
  const [lang, setLang] = useState<Lang>(() => store.get<Lang>("lang", navigator.language.startsWith("bn") ? "bn" : "en"));
  const t = useMemo(() => makeT(lang), [lang]);
  const [recent, setRecent] = useState<string[]>(() => store.get("recent", []));
  const [archive, setArchive] = useState<{ info: ArchiveInfo; password?: string } | null>(null);
  const [pack, setPack] = useState<{ inputs: string[]; preset?: PackPreset } | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [toast, setToast] = useState<ToastData | null>(null);
  const [pwPrompt, setPwPrompt] = useState<PwPrompt | null>(null);
  const [dragging, setDragging] = useState(false);
  const [skipPrograms, setSkipPrograms] = useState(true);
  const [sfxOk, setSfxOk] = useState(false);
  const [shell, setShell] = useState<ShellStatus>({ supported: false, installed: false });
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const updateSettings = useCallback((patch: Partial<Settings>) => {
    setSettings((cur) => {
      const next = { ...cur, ...patch };
      saveSettings(next);
      return next;
    });
  }, []);
  useEffect(() => applyTheme(settings.theme), [settings.theme]);
  const [view, setView] = useState<View>({ kind: "home" });
  const [clouds, setClouds] = useState<CloudFolder[]>([]);
  const [shareServer, setShareServerState] = useState<string>(() => store.get("shareServer", ""));
  const setShareServer = (v: string) => {
    setShareServerState(v);
    store.set("shareServer", v);
  };
  const [shareTarget, setShareTarget] = useState<string | null>(null);
  const [convertTarget, setConvertTarget] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  // Folder you're in, per archive, so it survives a refresh after an edit.
  const [cwds, setCwds] = useState<Record<string, string>>({});
  const [confirm, setConfirm] = useState<null | { title: string; body?: string; confirm: string; danger?: boolean; run: () => void; extra?: "replace"; alt?: { label: string; run: () => void } }>(null);
  const [replaceOnAdd, setReplaceOnAdd] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [conflictAsk, setConflictAsk] = useState<null | { names: string[]; folder: string; resolve: (c: "skip" | "rename" | "overwrite" | null) => void }>(null);

  useEffect(() => {
    document.documentElement.lang = lang;
    store.set("lang", lang);
  }, [lang]);

  useEffect(() => {
    sfxAvailable().then(setSfxOk).catch(() => {});
    shellStatus().then(setShell).catch(() => {});
    cloudFolders().then(setClouds).catch(() => {});
  }, []);

  const remember = useCallback((p: string) => {
    setRecent((r) => {
      const next = [p, ...r.filter((x) => x !== p)].slice(0, 12);
      store.set("recent", next);
      return next;
    });
  }, []);

  const errorText = useCallback(
    (e: UiError) =>
      e.code === "wrong_password" ? t("wrongPw")
      : e.code === "too_large" ? t("tooLarge")
      : e.code === "no_space" ? t("noSpace", { need: human(Number(e.message.split("|")[0]), lang), free: human(Number(e.message.split("|")[1]), lang) })
      : e.code === "unsupported" ? t("unsupported")
      : e.code === "damaged" ? `${t("damaged")} ${e.message}`
      : e.code === "no_server" ? t("needServer")
      : e.message,
    [t, lang],
  );

  const failToast = useCallback(
    (err: unknown) => {
      const e = asUiError(err);
      setToast(e.code === "cancelled" ? { kind: "info", text: t("cancelled") } : { kind: "err", text: errorText(e) });
    },
    [t, errorText],
  );

  // One long job at a time: a right-click action that arrives during a job waits for it,
  // instead of taking over the progress window (which made the first job uncancellable).
  const serial = useRef<Promise<unknown>>(Promise.resolve());
  const exclusive = useCallback(<R,>(fn: () => Promise<R>): Promise<R> => {
    const p = serial.current.then(fn, fn);
    serial.current = p.catch(() => undefined);
    return p;
  }, []);
  const clearToast = useCallback(() => setToast(null), []);

  /** Run any long task behind the progress overlay; errors become toasts. */
  const runJob = useCallback(
    <R,>(title: string, fn: (jobId: string) => Promise<R>): Promise<R | undefined> =>
      exclusive(async () => {
        const id = uid();
        setJob({ id, kind: "pack", title, progress: null });
        try {
          return await fn(id);
        } catch (err) {
          failToast(err);
          return undefined;
        } finally {
          setJob((j) => (j?.id === id ? null : j));
        }
      }),
    [failToast, exclusive],
  );

  /** Ask for a password, then retry with it. */
  const askPassword = useCallback(
    (path: string, retry: (pw: string) => void, wrong = false, cancel?: () => void) =>
      setPwPrompt({ path, error: wrong ? t("wrongPw") : undefined, then: retry, cancel }),
    [t],
  );



  const openPath = useCallback(
    async (path: string, password?: string) => {
      try {
        const info = await inspectArchive(path, password);
        if (info.encrypted && !password) {
          askPassword(path, (pw) => openPath(path, pw));
          return;
        }
        setPwPrompt(null);
        setArchive({ info, password });
        setSkipPrograms(settingsRef.current.skipPrograms);
        remember(path);
      } catch (err) {
        const e = asUiError(err);
        if (e.code === "password_required" || (e.code === "wrong_password" && !password)) askPassword(path, (pw) => openPath(path, pw));
        else if (e.code === "wrong_password") askPassword(path, (pw) => openPath(path, pw), true);
        else failToast(e);
      }
    },
    [remember, askPassword, failToast],
  );

  const startPack = useCallback(
    async (req: PackRequest) => {
      setPack(null);
      await exclusive(async () => {
      const id = uid();
      setJob({ id, kind: "pack", progress: null });
      try {
        const r = await packFiles(id, req);
        const savedBytes = Math.max(0, r.input_bytes - r.output_bytes);
        remember(r.outputs[0]);
        setToast({
          kind: "ok",
          text: req.sfx
            ? t("sfxToast", { name: baseName(r.outputs[0]) })
            : savedBytes > 0 ? t("zippedToast", { saved: human(savedBytes, lang) }) : t("zippedToastNoSave", { n: num(r.files, lang) }),
          sub: r.stored_as_is ? t("storedNote", { n: num(r.stored_as_is, lang) }) : undefined,
          action: { label: t("showInFolder"), run: () => revealItemInDir(r.outputs[0]) },
        });
      } catch (err) {
        failToast(err);
      } finally {
        setJob((j) => (j?.id === id ? null : j));
      }
      });
    },
    [remember, t, lang, failToast, exclusive],
  );

  /** Extract one archive. `dest` defaults to the smart "Extract here" folder. Returns false if it stopped. */
  const runExtract = useCallback(
    async (path: string, opts: { password?: string; only?: string[]; parent?: string; allowPrograms?: boolean; title?: string } = {}): Promise<boolean> => {
      let dest: string;
      let result: string;
      let strip: string | null = null;
      try {
        if (opts.parent) {
          dest = `${opts.parent}${sep(path)}${stripArchiveExt(baseName(path))}`;
          result = dest;
        } else {
          ({ dest, result, strip } = await suggestExtractDest(path, opts.password));
        }
      } catch (err) {
        const e = asUiError(err);
        if (e.code === "password_required" || e.code === "wrong_password") {
          return new Promise((resolve) =>
            askPassword(path, (pw) => {
              setPwPrompt(null);
              runExtract(path, { ...opts, password: pw }).then(resolve);
            }, e.code === "wrong_password" && !!opts.password, () => resolve(false)),
          );
        }
        failToast(e);
        return false;
      }
      // Ask once if files already exist at the destination.
      let onConflict: "skip" | "rename" | "overwrite" = "skip";
      try {
        const clash = await extractConflicts({ archive: path, dest, password: opts.password, only: opts.only ?? [], strip });
        if (clash.length) {
          const choice = await new Promise<"skip" | "rename" | "overwrite" | null>((resolve) => setConflictAsk({ names: clash, folder: baseName(result), resolve }));
          setConflictAsk(null);
          if (!choice) return false;
          onConflict = choice;
        }
      } catch {
        /* the extraction itself will report real problems */
      }
      return exclusive(async (): Promise<boolean> => {
      const id = uid();
      setJob({ id, kind: "extract", title: opts.title, progress: null });
      try {
        const r = await extractArchive(id, {
          archive: path,
          dest,
          password: opts.password ?? null,
          overwrite: false,
          allowPrograms: opts.allowPrograms ?? true,
          only: opts.only ?? [],
          strip,
          onConflict,
        });
        remember(path);
        const prefs = settingsRef.current;
        if (prefs.openAfterExtract) revealItemInDir(result).catch(() => {});
        let trashed = false;
        // Only when everything came out fine, the whole archive was extracted, and it's a single file.
        if (prefs.trashAfterExtract && r.skipped.length === 0 && !(opts.only ?? []).length && !/\.\d{3}$/.test(path)) {
          try {
            await trashPath(path);
            trashed = true;
            setRecent((rs) => {
              const next = rs.filter((x) => x !== path);
              store.set("recent", next);
              return next;
            });
            if (archiveRef.current?.info.path === path) setArchive(null);
          } catch {
            /* leave it in place */
          }
        }
        setToast({
          kind: "ok",
          text: t("extractedToast", { n: num(r.files, lang), dest: baseName(result) }) + (trashed ? ` ${t("movedToTrash")}` : ""),
          sub: r.skipped.length ? `${t("skippedNote", { n: num(r.skipped.length, lang) })}: ${r.skipped.slice(0, 3).map(([n, why]) => `${baseName(n)} (${why})`).join(", ")}` : undefined,
          action: { label: t("showInFolder"), run: () => revealItemInDir(result) },
        });
        return true;
      } catch (err) {
        const e = asUiError(err);
        setJob(null);
        if (e.code === "password_required" || e.code === "wrong_password") {
          // Ask outside the job lock so the next job isn't held up by the dialog.
          setTimeout(() => askPassword(path, (pw) => {
            setPwPrompt(null);
            runExtract(path, { ...opts, password: pw });
          }, e.code === "wrong_password" && !!opts.password), 0);
        } else {
          failToast(e);
        }
        return false;
      } finally {
        setJob((j) => (j?.id === id ? null : j));
      }
      });
    },
    [askPassword, failToast, remember, t, lang, exclusive],
  );

  // Right-click menu and "Open with Zipkoo".
  const runShellAction = useCallback(
    async (a: ShellAction) => {
      const paths = a.paths;
      switch (a.action) {
        case "zip-here": {
          const output = await suggestPackOutput(paths, "zip");
          if (output) await startPack({ inputs: paths, output, format: "zip", level: settingsRef.current.defaultLevel, smart: settingsRef.current.smart, password: null, hideNames: false, splitSize: null, sfx: false });
          break;
        }
        case "lock":
          setPack({ inputs: paths, preset: { lock: true } });
          break;
        case "sfx":
          setPack({ inputs: paths, preset: { sfx: true } });
          break;
        case "extract-here": {
          const archives = paths.filter(isArchive);
          for (let i = 0; i < archives.length; i++) {
            await runExtract(archives[i], { title: archives.length > 1 ? t("extractingN", { i: num(i + 1, lang), n: num(archives.length, lang) }) : undefined });
          }
          break;
        }
        case "open":
          await openPath(paths[0]);
          break;
        default:
          if (paths.length === 1 && isArchive(paths[0])) await openPath(paths[0]);
          else setPack({ inputs: paths });
      }
    },
    [startPack, runExtract, openPath, t, lang],
  );

  // Process menu clicks one at a time, in order.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const drain = useCallback(() => {
    queue.current = queue.current.then(async () => {
      const actions = await takeShellActions().catch(() => [] as ShellAction[]);
      for (const a of actions) await runShellAction(a);
    });
  }, [runShellAction]);

  const archiveRef = useRef(archive);
  archiveRef.current = archive;
  const cwdsRef = useRef(cwds);
  cwdsRef.current = cwds;

  /** Apply a change to the open archive, then reload it in place. */
  const applyEdit = useCallback(
    async (change: Change) => {
      const a = archiveRef.current;
      if (!a) return;
      const r = await runJob(t("editing"), (id) => editArchive(id, a.info.path, change, a.password));
      if (!r) return;
      setToast({ kind: "ok", text: t("edited", { added: num(r.added + r.replaced, lang), deleted: num(r.deleted, lang), renamed: num(r.renamed, lang) }), sub: r.skipped.length ? `${t("skippedNote", { n: num(r.skipped.length, lang) })}: ${r.skipped.slice(0, 3).map(baseName).join(", ")}` : undefined });
      await openPath(a.info.path, a.password);
      setRevision((n) => n + 1);
    },
    [runJob, t, lang, openPath],
  );

  const handlePaths = useCallback(
    (paths: string[]) => {
      if (!paths.length) return;
      const a = archiveRef.current;
      // Dropped onto an open archive that can be changed: offer to add them to the current folder.
      if (a && EDITABLE.includes(a.info.format) && !(paths.length === 1 && paths[0] === a.info.path)) {
        const folder = cwdsRef.current[a.info.path] ?? "";
        setReplaceOnAdd(false);
        setConfirm({
          title: t("addTitle", { n: num(paths.length, lang), where: folder ? folder.slice(0, -1).split("/").pop()! : baseName(a.info.path) }),
          confirm: t("addHere"),
          extra: "replace",
          run: () => applyEdit({ kind: "add", inputs: paths, folder, replace: replaceRef.current }),
          alt: { label: t("newArchiveInstead"), run: () => setPack({ inputs: paths }) },
        });
        return;
      }
      if (paths.length === 1 && isArchive(paths[0])) openPath(paths[0]);
      else setPack({ inputs: paths });
    },
    [openPath, applyEdit, t, lang],
  );
  const replaceRef = useRef(replaceOnAdd);
  replaceRef.current = replaceOnAdd;

  const openFromArchive = useCallback(
    async (e: Entry, force = false) => {
      const a = archiveRef.current;
      if (!a) return;
      if (e.executable && !force) {
        setConfirm({ title: t("programOpenTitle"), body: t("programOpenBody", { name: baseName(e.name) }), confirm: t("openAnyway"), danger: true, run: () => openFromArchive(e, true) });
        return;
      }
      try {
        const p = await openEntry(a.info.path, e.name, a.password);
        await osOpen(p);
      } catch (err) {
        failToast(err);
      }
    },
    [t, failToast],
  );

  const dragOut = useCallback(
    async (names: string[]) => {
      const a = archiveRef.current;
      if (!a || !names.length) return;
      try {
        const d = await prepareDrag(a.info.path, names, a.password);
        await startDrag({ item: d.items, icon: d.icon });
      } catch (err) {
        failToast(err);
      }
    },
    [failToast],
  );

  useEffect(() => {
    const un = onProgress((id, p) => setJob((j) => (j && j.id === id ? { ...j, progress: p } : j)));
    const dd = getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragging(true);
      else if (p.type === "leave") setDragging(false);
      else if (p.type === "drop") {
        setDragging(false);
        handlePaths(p.paths);
      }
    });
    const sa = onShellActions(drain);
    drain();
    return () => {
      un.then((f) => f());
      dd.then((f) => f());
      sa.then((f) => f());
    };
  }, [handlePaths, drain]);

  const pick = async (kind: "files" | "folder" | "archive") => {
    const res = await openDialog({
      multiple: kind === "files",
      directory: kind === "folder",
      filters: kind === "archive" ? [{ name: "Archives", extensions: ["zip", "7z", "rar", "tar", "gz", "tgz", "zst", "tzst", "bz2", "xz", "001", "exe"] }] : undefined,
    });
    if (!res) return;
    const paths = Array.isArray(res) ? res : [res];
    if (kind === "archive") openPath(paths[0]);
    else setPack({ inputs: paths });
  };

  const extractFromView = async (only: string[], chooseDest: boolean) => {
    if (!archive) return;
    let parent: string | undefined;
    if (chooseDest || settings.extractAsk) {
      const d = await openDialog({ directory: true, defaultPath: dirName(archive.info.path) });
      if (!d || Array.isArray(d)) return;
      parent = d;
    }
    await runExtract(archive.info.path, { password: archive.password, only, parent, allowPrograms: !skipPrograms });
  };

  const checkUpdates = useCallback(
    async (silent = false): Promise<string> => {
      try {
        const u = await checkUpdate();
        if (!u) return t("upToDate");
        setToast({
          kind: "info",
          text: t("updateAvailable", { v: u.version }),
          action: {
            label: t("installRestart"),
            run: () => {
              runJob(t("working"), async () => {
                await u.downloadAndInstall();
                await relaunch();
              });
            },
          },
        });
        return t("updateAvailable", { v: u.version });
      } catch (e) {
        const msg = t("updateFailed", { msg: asUiError(e).message });
        if (!silent) return msg;
        return msg;
      }
    },
    [t, runJob],
  );
  useEffect(() => {
    // Quietly look for an update once at start (direct downloads with signing set up only).
    appInfo()
      .then((i) => {
        if (i.flavor === "direct" && i.updater_ready && settingsRef.current.autoUpdate) checkUpdates(true);
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleShell = async () => {
    try {
      setShell(await shellSet(!shell.installed));
    } catch (err) {
      failToast(err);
    }
  };

  const ctx: ViewCtx = useMemo(
    () => ({ t, lang, toast: setToast, runJob, openPath: (p: string) => openPath(p), fail: failToast }),
    [t, lang, runJob, openPath, failToast],
  );

  const go = (v: View) => {
    setArchive(null);
    setView(v);
  };

  const createShare = async (hours: number, downloads: number | null, password: string | null) => {
    const path = shareTarget;
    setShareTarget(null);
    if (!path) return;
    if (!shareServer) {
      setToast({ kind: "err", text: t("needServer"), action: { label: t("navShares"), run: () => go({ kind: "shares" }) } });
      return;
    }
    const rec = await runJob(t("sharing"), (id) => shareFile(id, path, shareServer, hours, downloads, password));
    if (!rec) return;
    let copied = false;
    try {
      await navigator.clipboard.writeText(rec.url);
      copied = true;
    } catch {
      /* clipboard refused */
    }
    setToast({ kind: "ok", text: copied ? t("shareDone") : rec.name, sub: rec.url, action: { label: t("navShares"), run: () => go({ kind: "shares" }) } });
  };

  const updateCurrent = async () => {
    if (!archive) return;
    const res = await openDialog({ directory: true, title: t("updateHint"), defaultPath: dirName(archive.info.path) });
    if (!res || Array.isArray(res)) return;
    const { info, password } = archive;
    const r = await runJob(t("updating"), (id) => updateArchive(id, info.path, [res], password));
    if (!r) return;
    setToast({ kind: "ok", text: t("updateDone", { v: num(r.version, lang), added: num(r.added, lang), changed: num(r.changed, lang), removed: num(r.removed, lang), unchanged: num(r.unchanged, lang) }) });
    await openPath(info.path, password);
    setRevision((n) => n + 1);
  };

  const convertTo = async (format: "zip" | "seven-z" | "tar-zst") => {
    const src = convertTarget;
    setConvertTarget(null);
    if (!src) return;
    const r = await runJob(t("converting"), (id) => convertArchives(id, [src], format, archive?.password));
    const first = r?.[0];
    if (!first) return;
    if (first.output) setToast({ kind: "ok", text: `${baseName(src)} → ${baseName(first.output)}`, action: { label: t("showInFolder"), run: () => revealItemInDir(first.output!) } });
    else setToast({ kind: "err", text: first.error ?? t("damaged") });
  };

  return (
    <div className={`app ${dragging ? "dragging" : ""}`}>
      <nav className="side" aria-label="Zipkoo">
        <div className="brand">
          <Mark size={30} />
          <span className="wordmark">zipkoo</span>
        </div>
        {([
          ["home", "home", t("home")],
          ["search", "search", t("navSearch")],
          ["watch", "clock", t("navWatch")],
          ["shares", "lock", t("navShares")],
          ["tools", "shield", t("navTools")],
          ["settings", "settings", t("navSettings")],
        ] as const).map(([k, icon, label]) => (
          <button key={k} className={`nav ${!archive && view.kind === k ? "on" : ""}`} onClick={() => go({ kind: k })}>
            <Icon name={icon} />
            {label}
          </button>
        ))}
        {clouds.length > 0 && <h2 className="side-label">{t("navCloud")}</h2>}
        {clouds.map((c) => (
          <button key={c.path} className={`nav file ${!archive && view.kind === "cloud" && view.folder.path === c.path ? "on" : ""}`} title={c.path} onClick={() => go({ kind: "cloud", folder: c })}>
            <Icon name="globe" size={16} />
            <span className="ellipsis">{c.label}</span>
          </button>
        ))}
        <h2 className="side-label">{t("recent")}</h2>
        <div className="side-recent">
          {recent.slice(0, 8).map((p) => (
            <button key={p} className={`nav file ${archive?.info.path === p ? "on" : ""}`} title={p} onClick={() => openPath(p)}>
              <Icon name="clock" size={16} />
              <span className="ellipsis">{baseName(p)}</span>
            </button>
          ))}
        </div>
        <div className="side-foot">
          <button className="btn quiet small" onClick={() => setLang(lang === "en" ? "bn" : "en")}>
            <Icon name="globe" size={16} />
            {t("lang")}
          </button>
          <p className="motto">{t("tagline")}</p>
        </div>
      </nav>

      <main className="content">
        {archive ? (
          <ArchiveView
            key={archive.info.path}
            t={t}
            lang={lang}
            info={archive.info}
            password={archive.password}
            skipPrograms={skipPrograms}
            setSkipPrograms={setSkipPrograms}
            onExtract={extractFromView}
            onClose={() => setArchive(null)}
            ctx={ctx}
            onShare={() => setShareTarget(archive.info.path)}
            onUpdate={updateCurrent}
            onConvert={() => setConvertTarget(archive.info.path)}
            revision={revision}
            cwd={cwds[archive.info.path] ?? ""}
            setCwd={(c) => setCwds((m) => ({ ...m, [archive.info.path]: c }))}
            editable={EDITABLE.includes(archive.info.format)}
            onOpenEntry={(e) => openFromArchive(e)}
            onDragOut={dragOut}
            onDelete={(names) =>
              setConfirm({
                title: t("deleteTitle", { n: num(names.length, lang), name: baseName(archive.info.path) }),
                body: t("deleteBody"),
                confirm: t("deleteBtn"),
                danger: true,
                run: () => applyEdit({ kind: "delete", names }),
              })
            }
            onRename={(name) => setRenaming(name)}
          />
        ) : view.kind === "search" ? (
          <SearchView ctx={ctx} extra={recent} />
        ) : view.kind === "watch" ? (
          <WatchView ctx={ctx} />
        ) : view.kind === "shares" ? (
          <SharesView ctx={ctx} server={shareServer} setServer={setShareServer} askPassword={(name, retry, wrong) => setPwPrompt({ path: name, error: wrong ? t("wrongPw") : undefined, then: (pw) => { setPwPrompt(null); retry(pw); } })} />
        ) : view.kind === "settings" ? (
          <SettingsView t={t} lang={lang} setLang={setLang} settings={settings} update={updateSettings} shell={shell} toggleShell={toggleShell} shareServer={shareServer} setShareServer={setShareServer} checkUpdates={() => checkUpdates(false)} />
        ) : view.kind === "tools" ? (
          <ToolsView ctx={ctx} />
        ) : view.kind === "cloud" ? (
          <FolderView ctx={ctx} folder={view.folder} />
        ) : (
          <Home t={t} recent={recent} onPackFiles={() => pick("files")} onPackFolder={() => pick("folder")} onOpen={() => pick("archive")} onOpenPath={openPath} />
        )}
      </main>

      {dragging && (
        <div className="drag-veil" aria-hidden="true">
          <Mark size={64} />
          <span>{t("dragHere")}</span>
        </div>
      )}
      {pack && <PackSheet t={t} lang={lang} inputs={pack.inputs} preset={pack.preset} sfxEnabled={sfxOk} clouds={clouds} defaults={{ format: settings.defaultFormat, level: settings.defaultLevel, smart: settings.smart }} onCancel={() => setPack(null)} onStart={startPack} />}
      {confirm && (
        <ConfirmDialog
          title={confirm.title}
          body={confirm.body}
          confirm={confirm.confirm}
          cancel={t("cancel")}
          danger={confirm.danger}
          onCancel={() => setConfirm(null)}
          onConfirm={() => { const c = confirm; setConfirm(null); c.run(); }}
        >
          {confirm.extra === "replace" && (
            <label className="check small">
              <input id="add-replace" type="checkbox" checked={replaceOnAdd} onChange={(e) => setReplaceOnAdd(e.target.checked)} />
              {t("addReplace")}
            </label>
          )}
          {confirm.alt && <button className="link" onClick={() => { const c = confirm; setConfirm(null); c.alt!.run(); }}>{confirm.alt.label}</button>}
        </ConfirmDialog>
      )}
      {conflictAsk && (
        <div className="scrim" role="alertdialog" aria-modal="true" aria-labelledby="conflict-title" onKeyDown={(e) => e.key === "Escape" && conflictAsk.resolve(null)}>
          <div className="sheet narrow">
            <h2 id="conflict-title">{t("conflictTitle", { n: num(conflictAsk.names.length, lang) })}</h2>
            <p>{t("conflictBody", { folder: conflictAsk.folder })}</p>
            <ul className="inputs">
              {conflictAsk.names.slice(0, 5).map((n) => <li key={n} title={n}>{baseName(n)}</li>)}
              {conflictAsk.names.length > 5 && <li className="muted">+{num(conflictAsk.names.length - 5, lang)}</li>}
            </ul>
            <div className="row gap end">
              <button className="btn quiet" onClick={() => conflictAsk.resolve(null)}>{t("cancel")}</button>
              <button className="btn" onClick={() => conflictAsk.resolve("skip")}>{t("skipThem")}</button>
              <button className="btn" onClick={() => conflictAsk.resolve("overwrite")}>{t("replaceAll")}</button>
              <button className="btn primary" autoFocus onClick={() => conflictAsk.resolve("rename")}>{t("keepBoth")}</button>
            </div>
          </div>
        </div>
      )}
      {renaming && archive && (
        <PromptDialog
          title={t("renameTitle", { name: renaming.replace(/\/$/, "").split("/").pop()! })}
          label={t("newName")}
          initial={renaming.replace(/\/$/, "").split("/").pop()!}
          submit={t("renameBtn")}
          cancel={t("cancel")}
          validate={(v) => (/[\\/:*?"<>|]/.test(v) || v === "." || v === ".." ? t("nameInvalid") : null)}
          onCancel={() => setRenaming(null)}
          onSubmit={(v) => {
            const from = renaming;
            setRenaming(null);
            const isDir = from.endsWith("/");
            const base = from.replace(/\/$/, "");
            const parent = base.includes("/") ? base.slice(0, base.lastIndexOf("/") + 1) : "";
            applyEdit({ kind: "rename", from, to: parent + v + (isDir ? "/" : "") });
          }}
        />
      )}
      {shareTarget && <ShareSheet t={t} name={baseName(shareTarget)} onCancel={() => setShareTarget(null)} onCreate={createShare} />}
      {convertTarget && (
        <div className="scrim" role="dialog" aria-modal="true" aria-labelledby="conv-title">
          <div className="sheet narrow">
            <h2 id="conv-title">{t("convertThis").replace("…", "")}: {baseName(convertTarget)}</h2>
            <p>{t("convertSub")}</p>
            <div className="row gap">
              {(["zip", "seven-z", "tar-zst"] as const).map((f) => (
                <button key={f} className="btn" onClick={() => convertTo(f)}>{({ zip: "ZIP", "seven-z": "7Z", "tar-zst": "TAR.ZST" } as const)[f]}</button>
              ))}
            </div>
            <div className="row end"><button className="btn quiet" onClick={() => setConvertTarget(null)}>{t("cancel")}</button></div>
          </div>
        </div>
      )}
      {pwPrompt && (
        <PasswordDialog t={t} name={baseName(pwPrompt.path)} error={pwPrompt.error} onSubmit={(pw) => pwPrompt.then(pw)} onCancel={() => { pwPrompt.cancel?.(); setPwPrompt(null); }} />
      )}
      {job && (
        <ProgressOverlay t={t} lang={lang} title={job.title ?? (job.kind === "pack" ? t("working") : t("extracting"))} progress={job.progress} onCancel={() => cancelJob(job.id)} />
      )}
      {toast && <Toast toast={toast} onDone={clearToast} />}
    </div>
  );
}
