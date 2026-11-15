import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
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
import { PasswordDialog, ProgressOverlay, Toast, type ToastData } from "./components/Overlays";

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
type PwPrompt = { path: string; error?: string; then: (pw: string) => void };

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

  useEffect(() => {
    document.documentElement.lang = lang;
    store.set("lang", lang);
  }, [lang]);

  useEffect(() => {
    sfxAvailable().then(setSfxOk).catch(() => {});
    shellStatus().then(setShell).catch(() => {});
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

  /** Ask for a password, then retry with it. */
  const askPassword = useCallback(
    (path: string, retry: (pw: string) => void, wrong = false) => setPwPrompt({ path, error: wrong ? t("wrongPw") : undefined, then: retry }),
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
        setSkipPrograms(info.executables.length > 0);
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
        setJob(null);
      }
    },
    [remember, t, lang, failToast],
  );

  /** Extract one archive. `dest` defaults to the smart "Extract here" folder. Returns false if it stopped. */
  const runExtract = useCallback(
    async (path: string, opts: { password?: string; only?: string[]; parent?: string; allowPrograms?: boolean; title?: string } = {}): Promise<boolean> => {
      let dest: string;
      let result: string;
      try {
        if (opts.parent) {
          dest = `${opts.parent}${sep(path)}${stripArchiveExt(baseName(path))}`;
          result = dest;
        } else {
          ({ dest, result } = await suggestExtractDest(path, opts.password));
        }
      } catch (err) {
        const e = asUiError(err);
        if (e.code === "password_required" || e.code === "wrong_password") {
          return new Promise((resolve) =>
            askPassword(path, (pw) => {
              setPwPrompt(null);
              runExtract(path, { ...opts, password: pw }).then(resolve);
            }, e.code === "wrong_password" && !!opts.password),
          );
        }
        failToast(e);
        return false;
      }
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
        });
        remember(path);
        setToast({
          kind: "ok",
          text: t("extractedToast", { n: num(r.files, lang), dest: baseName(result) }),
          sub: r.skipped.length ? `${t("skippedNote", { n: num(r.skipped.length, lang) })}: ${r.skipped.slice(0, 3).map(([n, why]) => `${baseName(n)} (${why})`).join(", ")}` : undefined,
          action: { label: t("showInFolder"), run: () => revealItemInDir(result) },
        });
        return true;
      } catch (err) {
        const e = asUiError(err);
        setJob(null);
        if (e.code === "password_required" || e.code === "wrong_password") {
          return new Promise((resolve) =>
            askPassword(path, (pw) => {
              setPwPrompt(null);
              runExtract(path, { ...opts, password: pw }).then(resolve);
            }, e.code === "wrong_password" && !!opts.password),
          );
        }
        failToast(e);
        return false;
      } finally {
        setJob((j) => (j?.id === id ? null : j));
      }
    },
    [askPassword, failToast, remember, t, lang],
  );

  // Right-click menu and "Open with Zipkoo".
  const runShellAction = useCallback(
    async (a: ShellAction) => {
      const paths = a.paths;
      switch (a.action) {
        case "zip-here": {
          const output = await suggestPackOutput(paths, "zip");
          if (output) await startPack({ inputs: paths, output, format: "zip", level: "normal", smart: true, password: null, hideNames: false, splitSize: null, sfx: false });
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

  const handlePaths = useCallback(
    (paths: string[]) => {
      if (!paths.length) return;
      if (paths.length === 1 && isArchive(paths[0])) openPath(paths[0]);
      else setPack({ inputs: paths });
    },
    [openPath],
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
    if (chooseDest) {
      const d = await openDialog({ directory: true, defaultPath: dirName(archive.info.path) });
      if (!d || Array.isArray(d)) return;
      parent = d;
    }
    await runExtract(archive.info.path, { password: archive.password, only, parent, allowPrograms: !skipPrograms });
  };

  const toggleShell = async () => {
    try {
      setShell(await shellSet(!shell.installed));
    } catch (err) {
      failToast(err);
    }
  };

  return (
    <div className={`app ${dragging ? "dragging" : ""}`}>
      <nav className="side" aria-label="Zipkoo">
        <div className="brand">
          <Mark size={30} />
          <span className="wordmark">zipkoo</span>
        </div>
        <button className={`nav ${!archive ? "on" : ""}`} onClick={() => setArchive(null)}>
          <Icon name="home" />
          {t("home")}
        </button>
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
          {shell.supported && (
            <label className="switch side-switch" title={shell.installed ? t("rightClickOn") : t("rightClickOff")}>
              <input id="shell-menu" type="checkbox" role="switch" checked={shell.installed} onChange={toggleShell} />
              <span className="track" aria-hidden="true" />
              {t("rightClick")}
            </label>
          )}
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
            t={t}
            lang={lang}
            info={archive.info}
            password={archive.password}
            skipPrograms={skipPrograms}
            setSkipPrograms={setSkipPrograms}
            onExtract={extractFromView}
            onClose={() => setArchive(null)}
          />
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
      {pack && <PackSheet t={t} lang={lang} inputs={pack.inputs} preset={pack.preset} sfxEnabled={sfxOk} onCancel={() => setPack(null)} onStart={startPack} />}
      {pwPrompt && (
        <PasswordDialog t={t} name={baseName(pwPrompt.path)} error={pwPrompt.error} onSubmit={(pw) => pwPrompt.then(pw)} onCancel={() => setPwPrompt(null)} />
      )}
      {job && (
        <ProgressOverlay t={t} lang={lang} title={job.title ?? (job.kind === "pack" ? t("working") : t("extracting"))} progress={job.progress} onCancel={() => cancelJob(job.id)} />
      )}
      {toast && <Toast toast={toast} onDone={() => setToast(null)} />}
    </div>
  );
}
