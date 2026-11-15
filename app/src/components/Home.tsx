import { Mark } from "./Mark";
import { Icon } from "./Icon";
import type { makeT } from "../i18n";
import { baseName, dirName } from "../util";

interface Props {
  t: ReturnType<typeof makeT>;
  recent: string[];
  onPackFiles: () => void;
  onPackFolder: () => void;
  onOpen: () => void;
  onOpenPath: (p: string) => void;
}

export function Home({ t, recent, onPackFiles, onPackFolder, onOpen, onOpenPath }: Props) {
  return (
    <div className="home">
      <section className="dock" aria-labelledby="home-title">
        <div className="dock-mark"><Mark size={64} /></div>
        <h1 id="home-title">{t("homeTitle")}</h1>
        <p className="lede">{t("dropSub")}</p>
        <div className="dock-actions">
          <button className="btn primary" onClick={onPackFiles}><Icon name="pack" />{t("packFiles")}</button>
          <button className="btn" onClick={onPackFolder}><Icon name="folder" />{t("packFolder")}</button>
          <button className="btn" onClick={onOpen}><Icon name="extract" />{t("openArchive")}</button>
        </div>
      </section>

      <section className="recent" aria-labelledby="recent-title">
        <h2 id="recent-title">{t("recent")}</h2>
        {recent.length === 0 ? (
          <p className="empty">{t("noRecent")}</p>
        ) : (
          <ul>
            {recent.map((p) => (
              <li key={p}>
                <button className="recent-item" onClick={() => onOpenPath(p)} title={p}>
                  <span className="ficon t-arch"><Icon name="zip" size={18} /></span>
                  <span className="names">
                    <span className="leaf">{baseName(p)}</span>
                    <span className="path">{dirName(p)}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
