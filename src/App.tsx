import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";

interface ProgressPayload {
  current: number;
  total: number;
  file: string;
}

type AppState = "loading" | "ready" | "converting" | "done" | "error";

const WIDTH_OPTIONS = [
  { label: "1280 px", value: 1280 },
  { label: "960 px", value: 960 },
  { label: "640 px", value: 640 },
  { label: "320 px", value: 320 },
];

const VIDEO_EXTS = [".mp4", ".avi", ".mov", ".mkv", ".webm"];

export default function App() {
  const [appState, setAppState] = useState<AppState>("loading");
  const [files, setFiles] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);
  const [outputFolder, setOutputFolder] = useState<string | null>(null);
  const [width, setWidth] = useState(640);
  const [fps, setFps] = useState(12);
  const [quality, setQuality] = useState(90);
  const [progress, setProgress] = useState(0);
  const [statusText, setStatusText] = useState("Loading...");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("validate_gifski")
      .then((version) => {
        setAppState("ready");
        setStatusText(version || "Ready");
      })
      .catch((err: unknown) => {
        setAppState("error");
        setStatusText(String(err));
      });
  }, []);

  useEffect(() => {
    let cleanup: (() => void) | undefined;

    getCurrentWindow()
      .onDragDropEvent(async (event) => {
        const payload = event.payload;

        if (payload.type === "enter") {
          setDragging(true);
        } else if (payload.type === "leave") {
          setDragging(false);
        } else if (payload.type === "drop") {
          setDragging(false);
          const newFiles: string[] = [];

          for (const p of payload.paths) {
            const lower = p.toLowerCase();
            if (VIDEO_EXTS.some((ext) => lower.endsWith(ext))) {
              newFiles.push(p);
            } else {
              const scanned = await invoke<string[]>("scan_folder", { folder: p });
              newFiles.push(...scanned);
            }
          }

          setFiles((prev) => {
            const existing = new Set(prev);
            return [...prev, ...newFiles.filter((f) => !existing.has(f))];
          });
        }
      })
      .then((fn) => {
        cleanup = fn;
      });

    return () => {
      cleanup?.();
    };
  }, []);

  useEffect(() => {
    const progressUnsub = listen<ProgressPayload>("conversion-progress", (e) => {
      const { current, total, file } = e.payload;
      setProgress(Math.round((current / total) * 100));
      setStatusText(`${current} / ${total} — ${file}`);
    });

    const completeUnsub = listen("conversion-complete", () => {
      setAppState("done");
      setProgress(100);
      setStatusText("All done!");
      setFiles([]);
      setTimeout(() => {
        setAppState("ready");
        setProgress(0);
        setStatusText("Ready");
      }, 2500);
    });

    const errorUnsub = listen<string>("conversion-error", (e) => {
      setAppState("error");
      setErrorMsg(e.payload);
      setStatusText("Conversion failed");
    });

    return () => {
      progressUnsub.then((f) => f());
      completeUnsub.then((f) => f());
      errorUnsub.then((f) => f());
    };
  }, []);

  const removeFile = (path: string) => {
    setFiles((prev) => prev.filter((f) => f !== path));
  };

  const pickOutputFolder = async () => {
    const result = await open({ directory: true, multiple: false, title: "Select Output Folder" });
    if (typeof result === "string") setOutputFolder(result);
  };

  const startConversion = async () => {
    setAppState("converting");
    setProgress(0);
    setErrorMsg(null);
    try {
      await invoke("convert_videos", {
        settings: { files, output_folder: outputFolder, width, fps, quality },
      });
    } catch (err) {
      setAppState("error");
      setErrorMsg(String(err));
      setStatusText("Conversion failed");
    }
  };

  const isConverting = appState === "converting";
  const canConvert = appState === "ready" && files.length > 0;
  const basename = (p: string) => p.split(/[/\\]/).pop() ?? p;

  return (
    <div className="app">
      <header className="header">
        <h1 className="logo">SpecialGifskiX</h1>
        <span className={`status status--${appState}`}>{statusText}</span>
      </header>

      <div
        className={[
          "drop-zone",
          dragging ? "drop-zone--over" : "",
          files.length > 0 ? "drop-zone--filled" : "",
          isConverting ? "drop-zone--disabled" : "",
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {files.length === 0 ? (
          <div className="drop-hint">
            <div className="drop-icon">⬇</div>
            <p className="drop-label">Drop videos or a folder here</p>
            <p className="drop-sub">MP4 · MOV · AVI · MKV · WebM</p>
          </div>
        ) : (
          <ul className="file-list">
            {files.map((f) => (
              <li key={f} className="file-item">
                <span className="file-icon">▶</span>
                <span className="file-name" title={f}>
                  {basename(f)}
                </span>
                {!isConverting && (
                  <button className="remove-btn" onClick={() => removeFile(f)} title="Remove">
                    ✕
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="settings">
        <div className="setting-row">
          <label className="setting-label">Max Width</label>
          <select
            className="setting-select"
            value={width}
            onChange={(e) => setWidth(Number(e.target.value))}
            disabled={isConverting}
          >
            {WIDTH_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>

        <div className="setting-row">
          <label className="setting-label">
            FPS <span className="badge">{fps}</span>
          </label>
          <input
            type="range"
            className="slider"
            min={3}
            max={24}
            value={fps}
            onChange={(e) => setFps(Number(e.target.value))}
            disabled={isConverting}
          />
        </div>

        <div className="setting-row">
          <label className="setting-label">
            Quality <span className="badge">{quality}</span>
          </label>
          <input
            type="range"
            className="slider"
            min={1}
            max={100}
            value={quality}
            onChange={(e) => setQuality(Number(e.target.value))}
            disabled={isConverting}
          />
        </div>

        <div className="setting-row">
          <label className="setting-label">Output</label>
          <div className="folder-row">
            <span className="folder-path" title={outputFolder ?? ""}>
              {outputFolder ? basename(outputFolder) : "Same as source"}
            </span>
            <button className="btn-sm" onClick={pickOutputFolder} disabled={isConverting}>
              Browse
            </button>
            {outputFolder && (
              <button
                className="btn-sm btn-sm--danger"
                onClick={() => setOutputFolder(null)}
                disabled={isConverting}
              >
                ✕
              </button>
            )}
          </div>
        </div>
      </div>

      {(isConverting || appState === "done") && (
        <div className="progress-wrap">
          <div className="progress-track">
            <div className="progress-fill" style={{ width: `${progress}%` }} />
          </div>
          <span className="progress-label">{statusText}</span>
        </div>
      )}

      {errorMsg && appState === "error" && (
        <div className="error-box">{errorMsg}</div>
      )}

      <button className="convert-btn" onClick={startConversion} disabled={!canConvert}>
        {isConverting
          ? "Converting..."
          : files.length > 1
          ? `Convert ${files.length} videos to GIF`
          : "Convert to GIF"}
      </button>
    </div>
  );
}
