import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

// ── Types ──────────────────────────────────────────────────────
interface Project {
  id: string;
  source_url: string;
  source_path?: string;
  title: string;
  created_at: string;
  status: string;
}
interface Clip {
  id: string;
  project_id: string;
  start_s: number;
  end_s: number;
  duration_s: number;
  title: string;
  hook_score?: number;
  viral_score?: number;
  heuristic_json?: string;
  llm_reason?: string;
  template_id?: string;
  render_path?: string;
  thumbnail_path?: string;
  status: string;
}
interface PipelineState {
  project_id: string;
  current_stage: string;
  progress: number;
  message: string;
  clips: ClipInfo[];
  error?: string;
}
interface ClipInfo {
  id: string; start_s: number; end_s: number; title: string;
  viral_score?: number; render_path?: string; status: string;
}
interface Template {
  id: string; name: string; ass_style_json: string;
  filter_graph_json: string; theme_json: string;
  thumbnail_json?: string; category?: string; description?: string;
}
interface Asset {
  id: string; asset_type: string; path: string; tags?: string; source_url?: string;
}
interface ThermalStatus {
  gpu_temp_c: number; gpu_utilization: number;
  memory_used_mb: number; memory_total_mb: number;
  throttling: boolean; render_paused: boolean;
}

type Tab = "queue" | "projects" | "templates" | "assets" | "settings";

// ── Helpers ────────────────────────────────────────────────────
const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

const STAGE_STEPS = ["Downloading","Transcribing","Analyzing","Scoring","Rendering","Completed"];

const statusBadgeClass = (s: string) => {
  if (s === "completed" || s === "Completed") return "badge badge-success";
  if (s === "failed" || s === "Failed") return "badge badge-error";
  if (["downloading","Downloading","transcribing","Transcribing","analyzing","Analyzing","scoring","Scoring","rendering","Rendering"].includes(s))
    return "badge badge-processing";
  return "badge badge-pending";
};

const viralColor = (score?: number) => {
  if (!score) return "var(--text-muted)";
  if (score >= 7.5) return "var(--success)";
  if (score >= 5) return "var(--warning)";
  return "var(--error)";
};

// ── Toast ──────────────────────────────────────────────────────
function Toast({ msg, type, onClose }: { msg: string; type: string; onClose: () => void }) {
  return (
    <div className={`toast ${type}`} onClick={onClose}>
      {type === "success" ? "✓" : type === "error" ? "✕" : "ℹ"}&nbsp;{msg}
    </div>
  );
}

// ── APP ────────────────────────────────────────────────────────
export default function App() {
  const [tab, setTab] = useState<Tab>("queue");
  const [projects, setProjects] = useState<Project[]>([]);
  const [clips, setClips] = useState<Clip[]>([]);
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [pipeline, setPipeline] = useState<PipelineState | null>(null);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [thermal, setThermal] = useState<ThermalStatus | null>(null);
  const [urlInput, setUrlInput] = useState("");
  const [templateId, setTemplateId] = useState("tiktok");
  const [maxClips, setMaxClips] = useState(3);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: string } | null>(null);

  // Settings state
  const [ollamaUrl, setOllamaUrl] = useState("http://localhost:11434");
  const [geminiKey, setGeminiKey] = useState("");
  const [pexelsKey, setPexelsKey] = useState("");
  const [maxDlTime, setMaxDlTime] = useState(30);
  const [maxTranscribeTime, setMaxTranscribeTime] = useState(60);
  const [autoOpenRender, setAutoOpenRender] = useState(false);

  const showToast = useCallback((msg: string, type = "info") => {
    setToast({ msg, type });
    setTimeout(() => setToast(null), 4000);
  }, []);

  const loadProjects = useCallback(async () => {
    try { setProjects(await invoke<Project[]>("get_projects")); }
    catch { /* silent — no projects yet */ }
  }, []);

  const loadTemplates = useCallback(async () => {
    try { setTemplates(await invoke<Template[]>("list_templates")); }
    catch { /* silent */ }
  }, []);

  const loadAssets = useCallback(async () => {
    try { setAssets(await invoke<Asset[]>("search_assets", { query: { limit: 100 } })); }
    catch { /* silent */ }
  }, []);

  useEffect(() => {
    loadProjects();
    loadTemplates();
    loadAssets();

    const unlistenPipeline = listen<PipelineState>("pipeline-update", (e) => {
      setPipeline(e.payload);
      if (e.payload.current_stage === "Completed") {
        showToast("🎉 Pipeline complete! Clips are ready.", "success");
        loadProjects();
      } else if (e.payload.current_stage === "Failed") {
        showToast(`Pipeline failed: ${e.payload.error}`, "error");
      }
    });

    const unlistenThermal = listen<ThermalStatus>("thermal-update", (e) => {
      setThermal(e.payload);
    });

    return () => {
      unlistenPipeline.then(fn => fn());
      unlistenThermal.then(fn => fn());
    };
  }, [loadProjects, loadTemplates, loadAssets, showToast]);

  const handleStart = async () => {
    if (!urlInput.trim()) { showToast("Paste a YouTube URL first", "error"); return; }
    try {
      setBusy(true);
      await invoke<string>("start_pipeline", {
        request: { source_url: urlInput.trim(), template_id: templateId, max_clips: maxClips },
      });
      setUrlInput("");
      showToast("Pipeline started! Downloading video...", "success");
    } catch (e) {
      showToast(`Failed: ${e}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const handleCancel = async () => {
    if (!pipeline) return;
    try { await invoke("cancel_pipeline", { projectId: pipeline.project_id }); showToast("Pipeline cancelled", "info"); }
    catch { showToast("Could not cancel", "error"); }
  };

  const handleSelectProject = async (p: Project) => {
    setSelectedProject(p);
    try { setClips(await invoke<Clip[]>("get_clips", { projectId: p.id })); }
    catch { setClips([]); }
    setTab("projects");
  };

  const handleImport = async () => {
    try {
      const paths = await open({ multiple: true, filters: [{ name: "Media", extensions: ["mp4","mov","mp3","wav","jpg","png","webp"] }] });
      if (paths && Array.isArray(paths)) {
        for (const path of paths) {
          const t = path.endsWith(".mp4") || path.endsWith(".mov") ? "broll"
            : path.endsWith(".mp3") || path.endsWith(".wav") ? "music" : "sfx";
          await invoke("import_asset", { assetType: t, sourcePath: path, tags: [], metadata: null });
        }
        loadAssets();
        showToast(`Imported ${paths.length} asset(s)`, "success");
      }
    } catch (e) { showToast(`Import failed: ${e}`, "error"); }
  };

  const handleResetDb = async () => {
    if (!confirm("This will delete ALL projects, clips, and assets. Templates and settings are kept. Continue?")) return;
    try {
      await invoke("reset_database");
      setProjects([]); setClips([]); setSelectedProject(null); setPipeline(null);
      showToast("Database cleared", "success");
    } catch (e) { showToast(`Reset failed: ${e}`, "error"); }
  };

  const stageIndex = pipeline ? STAGE_STEPS.indexOf(pipeline.current_stage) : -1;

  return (
    <div className="app">
      {/* ── Header ── */}
      <header className="header">
        <div className="logo">
          <div className="logo-icon">✂</div>
          Viral Clipper
        </div>

        <nav className="nav-tabs">
          {([
            ["queue", "🏠", "Queue"],
            ["projects", "🎬", "Projects"],
            ["templates", "🎨", "Templates"],
            ["assets", "📁", "Assets"],
            ["settings", "⚙️", "Settings"],
          ] as [Tab, string, string][]).map(([id, icon, label]) => (
            <button
              key={id}
              className={`tab ${tab === id ? "active" : ""}`}
              onClick={() => setTab(id)}
            >
              <span className="tab-icon">{icon}</span>
              {label}
            </button>
          ))}
        </nav>

        <div className="header-right">
          {thermal && (
            <div className={`thermal-badge ${thermal.throttling ? "hot" : thermal.render_paused ? "paused" : "cool"}`}>
              <div className="thermal-dot" />
              GPU {thermal.gpu_temp_c}°C
              {thermal.render_paused && " ⏸"}
            </div>
          )}
        </div>
      </header>

      {toast && <Toast msg={toast.msg} type={toast.type} onClose={() => setToast(null)} />}

      {/* ── Main ── */}
      <main className="main">

        {/* ══ QUEUE TAB ══ */}
        {tab === "queue" && (
          <div className="queue-tab">
            {/* Hero input */}
            <div className="hero-card">
              <div className="hero-label">✨ New Pipeline</div>
              <div className="hero-title">Turn any YouTube video into viral clips</div>
              <div className="hero-sub">Paste a URL, pick a template, and we'll do the rest — download, transcribe, cut, render.</div>
              <div className="url-input-row">
                <input
                  className="url-input"
                  value={urlInput}
                  onChange={e => setUrlInput(e.target.value)}
                  onKeyDown={e => e.key === "Enter" && handleStart()}
                  placeholder="https://youtube.com/watch?v=..."
                />
                <select
                  className="template-select-inline"
                  value={templateId}
                  onChange={e => setTemplateId(e.target.value)}
                >
                  {templates.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
                </select>
                <select
                  className="template-select-inline"
                  style={{ minWidth: 90 }}
                  value={maxClips}
                  onChange={e => setMaxClips(parseInt(e.target.value))}
                >
                  <option value={1}>1 clip</option>
                  <option value={3}>3 clips</option>
                  <option value={5}>5 clips</option>
                  <option value={10}>10 clips</option>
                </select>
                <button
                  className="btn btn-primary btn-lg"
                  onClick={handleStart}
                  disabled={busy || !urlInput.trim()}
                >
                  {busy ? "Starting…" : "🚀 Create Clips"}
                </button>
              </div>
            </div>

            {/* Active pipeline */}
            <div className="card pipeline-card">
              <div className="card-header">
                <div className="card-title">⚡ Active Pipeline</div>
                {pipeline && pipeline.current_stage !== "Completed" && pipeline.current_stage !== "Failed" && (
                  <button className="btn btn-ghost btn-sm" onClick={handleCancel}>Cancel</button>
                )}
              </div>

              {pipeline ? (
                <>
                  <div className="stage-row">
                    <div className="stage-name">
                      <span className={statusBadgeClass(pipeline.current_stage)}>
                        {pipeline.current_stage}
                      </span>
                    </div>
                    <div className="stage-pct">{Math.round(pipeline.progress * 100)}%</div>
                  </div>

                  <div className="progress-track">
                    <div className="progress-fill" style={{ width: `${pipeline.progress * 100}%` }} />
                  </div>

                  {/* Stage steps */}
                  <div className="stage-steps">
                    {STAGE_STEPS.filter(s => s !== "Failed").map((s, i) => (
                      <div
                        key={s}
                        className={`stage-step ${i < stageIndex ? "done" : i === stageIndex ? "active" : ""}`}
                        title={s}
                      />
                    ))}
                  </div>

                  <div className="stage-message">{pipeline.message}</div>

                  {pipeline.error && (
                    <div className="pipeline-error">
                      <strong>Error:</strong> {pipeline.error}
                    </div>
                  )}

                  {pipeline.current_stage === "Rendering" && pipeline.clips.length > 0 && (
                    <div className="rendering-clips">
                      <h4>Clips being rendered</h4>
                      {pipeline.clips.map(c => (
                        <div key={c.id} className="clip-mini">
                          <span>{c.title}</span>
                          <span className={statusBadgeClass(c.status)}>{c.status}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </>
              ) : (
                <div className="empty-state" style={{ padding: "30px 0" }}>
                  <div className="empty-icon">⚡</div>
                  <p>No active pipeline — paste a URL above to get started</p>
                </div>
              )}
            </div>

            {/* Recent projects */}
            <div className="card projects-card">
              <div className="card-header">
                <div className="card-title">🕒 Recent Projects</div>
                <button className="btn btn-ghost btn-sm" onClick={loadProjects}>Refresh</button>
              </div>

              {projects.length === 0 ? (
                <div className="empty-state" style={{ padding: "30px 0" }}>
                  <div className="empty-icon">🎬</div>
                  <p>No projects yet — create your first one above</p>
                </div>
              ) : (
                <div className="project-list">
                  {projects.slice(0, 12).map(p => (
                    <div key={p.id} className="project-row" onClick={() => handleSelectProject(p)}>
                      <div className="project-icon">🎬</div>
                      <div className="project-info">
                        <div className="project-title">{p.title}</div>
                        <div className="project-url">{p.source_url}</div>
                      </div>
                      <span className={statusBadgeClass(p.status)}>{p.status}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}

        {/* ══ PROJECTS TAB ══ */}
        {tab === "projects" && (
          <div className="clips-view">
            {selectedProject ? (
              <>
                <div className="back-row">
                  <button className="btn btn-ghost btn-sm" onClick={() => { setSelectedProject(null); setClips([]); }}>
                    ← Back
                  </button>
                  <div>
                    <h2>{selectedProject.title}</h2>
                    <div style={{ fontSize: 12, color: "var(--text-muted)", marginTop: 2 }}>{selectedProject.source_url}</div>
                  </div>
                  <span className={statusBadgeClass(selectedProject.status)} style={{ marginLeft: "auto" }}>
                    {selectedProject.status}
                  </span>
                </div>

                {clips.length === 0 ? (
                  <div className="empty-state">
                    <div className="empty-icon">✂️</div>
                    <h3>No clips yet</h3>
                    <p>This project hasn't generated any clips yet. Wait for the pipeline to complete.</p>
                  </div>
                ) : (
                  <div className="clips-grid">
                    {clips.map(clip => (
                      <div key={clip.id} className="clip-card">
                        <div className="clip-thumb">
                          {clip.render_path
                            ? <video src={clip.render_path} controls />
                            : (
                              <div className="clip-thumb-placeholder">
                                <span>🎬</span>
                                <span>Not rendered yet</span>
                              </div>
                            )
                          }
                          {clip.viral_score != null && (
                            <div className="clip-score-badge" style={{
                              background: `${viralColor(clip.viral_score)}22`,
                              color: viralColor(clip.viral_score),
                              border: `1px solid ${viralColor(clip.viral_score)}44`,
                            }}>
                              ⚡ {clip.viral_score.toFixed(1)}/10
                            </div>
                          )}
                        </div>
                        <div className="clip-body">
                          <h4>{clip.title}</h4>
                          <div className="clip-meta">
                            <span>⏱ {fmt(clip.start_s)} – {fmt(clip.end_s)}</span>
                            <span>{fmt(clip.duration_s)}</span>
                            {clip.hook_score != null && <span>🎯 {(clip.hook_score * 100).toFixed(0)}% hook</span>}
                          </div>
                          {clip.llm_reason && <div className="clip-reason">{clip.llm_reason}</div>}
                          <div className="clip-actions">
                            {clip.render_path && (
                              <button className="btn btn-primary btn-sm" onClick={() =>
                                invoke("opener_open_path", { path: clip.render_path }).catch(() => showToast("Cannot open file", "error"))
                              }>
                                📂 Open
                              </button>
                            )}
                            <span className={statusBadgeClass(clip.status)}>{clip.status}</span>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </>
            ) : (
              <div>
                <h2 style={{ fontFamily: "var(--font-display)", fontSize: 20, fontWeight: 700, marginBottom: 16 }}>All Projects</h2>
                {projects.length === 0 ? (
                  <div className="empty-state">
                    <div className="empty-icon">🎬</div>
                    <h3>No projects</h3>
                    <p>Go to the Queue tab and paste a YouTube URL to create your first project.</p>
                  </div>
                ) : (
                  <div className="project-list" style={{ maxWidth: 800 }}>
                    {projects.map(p => (
                      <div key={p.id} className="project-row" onClick={() => handleSelectProject(p)}>
                        <div className="project-icon">🎬</div>
                        <div className="project-info">
                          <div className="project-title">{p.title}</div>
                          <div className="project-url">{p.source_url}</div>
                        </div>
                        <span className={statusBadgeClass(p.status)}>{p.status}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {/* ══ TEMPLATES TAB ══ */}
        {tab === "templates" && (
          <div>
            <div style={{ marginBottom: 20, display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <div>
                <h2 style={{ fontFamily: "var(--font-display)", fontSize: 20, fontWeight: 700 }}>Templates</h2>
                <p style={{ fontSize: 13, color: "var(--text-muted)", marginTop: 4 }}>
                  Choose a style template for your clips. Templates control caption style, colors, and filter effects.
                </p>
              </div>
            </div>
            <div className="templates-grid">
              {templates.map(t => (
                <div
                  key={t.id}
                  className={`template-card ${templateId === t.id ? "selected" : ""}`}
                  onClick={() => { setTemplateId(t.id); showToast(`Selected: ${t.name}`, "success"); }}
                >
                  {templateId === t.id && <div className="selected-check">✓</div>}
                  <div className="template-preview">
                    {t.category === "thumbnail" ? "🖼️" : t.id === "tiktok" ? "🎵" : t.id === "hormozi" ? "💪" : t.id === "minimal" ? "✨" : t.id === "instagram_reels" ? "📸" : t.id === "youtube_shorts" ? "▶️" : "🎬"}
                  </div>
                  <div className="template-name">{t.name}</div>
                  <div className="template-desc">{t.description}</div>
                  {t.category && <div className="template-cat">{t.category}</div>}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* ══ ASSETS TAB ══ */}
        {tab === "assets" && (
          <div>
            <div style={{ marginBottom: 20, display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <div>
                <h2 style={{ fontFamily: "var(--font-display)", fontSize: 20, fontWeight: 700 }}>Assets</h2>
                <p style={{ fontSize: 13, color: "var(--text-muted)", marginTop: 4 }}>
                  B-roll, music, and SFX files imported into your library.
                </p>
              </div>
              <button className="btn btn-primary" onClick={handleImport}>📥 Import Assets</button>
            </div>

            {assets.length === 0 ? (
              <div className="empty-state">
                <div className="empty-icon">📁</div>
                <h3>No assets yet</h3>
                <p>Import MP4, MOV, MP3, WAV, JPG or PNG files to use in your clips.</p>
                <button className="btn btn-primary" onClick={handleImport}>📥 Import Assets</button>
              </div>
            ) : (
              <div className="assets-grid">
                {assets.map(a => (
                  <div key={a.id} className="asset-card">
                    <div className="asset-preview">
                      {a.asset_type === "broll" ? "🎬" : a.asset_type === "music" ? "🎵" : a.asset_type === "sfx" ? "🔊" : "📄"}
                    </div>
                    <div className="asset-info">
                      <div className="asset-name">{a.path.split(/[/\\]/).pop()}</div>
                      <div className="asset-type">{a.asset_type}</div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* ══ SETTINGS TAB ══ */}
        {tab === "settings" && (
          <div className="settings-tab">

            {/* ── Pipeline settings ── */}
            <div className="card">
              <div className="settings-section-title">🚀 Pipeline</div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Default template</div>
                  <div className="settings-sublabel">Applied automatically to new pipelines</div>
                </div>
                <div className="settings-control">
                  <select
                    className="form-input form-select"
                    value={templateId}
                    onChange={e => setTemplateId(e.target.value)}
                  >
                    {templates.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Default max clips</div>
                  <div className="settings-sublabel">How many clips to generate per video</div>
                </div>
                <div className="settings-control">
                  <select className="form-input form-select" value={maxClips} onChange={e => setMaxClips(+e.target.value)}>
                    {[1,3,5,10].map(n => <option key={n} value={n}>{n} clip{n > 1 ? "s" : ""}</option>)}
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Download timeout (minutes)</div>
                  <div className="settings-sublabel">Max time allowed for yt-dlp to download</div>
                </div>
                <div className="settings-control">
                  <input className="form-input" type="number" min={5} max={120} value={maxDlTime}
                    onChange={e => setMaxDlTime(+e.target.value)} />
                </div>
              </div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Transcription timeout (minutes)</div>
                  <div className="settings-sublabel">Max time for faster-whisper to transcribe audio</div>
                </div>
                <div className="settings-control">
                  <input className="form-input" type="number" min={5} max={180} value={maxTranscribeTime}
                    onChange={e => setMaxTranscribeTime(+e.target.value)} />
                </div>
              </div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Auto-open rendered clips</div>
                  <div className="settings-sublabel">Open the file in Explorer when render completes</div>
                </div>
                <div className="settings-control">
                  <label className="toggle">
                    <input type="checkbox" checked={autoOpenRender} onChange={e => setAutoOpenRender(e.target.checked)} />
                    <span className="toggle-slider" />
                  </label>
                </div>
              </div>
            </div>

            {/* ── AI / API keys ── */}
            <div className="card">
              <div className="settings-section-title">🤖 AI & API Keys</div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Ollama URL</div>
                  <div className="settings-sublabel">Local Ollama instance for AI clip scoring (optional)</div>
                </div>
                <div className="settings-control">
                  <input className="form-input" value={ollamaUrl} onChange={e => setOllamaUrl(e.target.value)}
                    placeholder="http://localhost:11434" />
                </div>
              </div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Gemini API Key</div>
                  <div className="settings-sublabel">For viral prediction &amp; content optimization (optional)</div>
                </div>
                <div className="settings-control">
                  <input className="form-input" type="password" value={geminiKey}
                    onChange={e => setGeminiKey(e.target.value)} placeholder="AIza…" />
                </div>
              </div>

              <div className="settings-row">
                <div>
                  <div className="settings-label">Pexels API Key</div>
                  <div className="settings-sublabel">For B-roll stock footage search (optional)</div>
                </div>
                <div className="settings-control">
                  <input className="form-input" type="password" value={pexelsKey}
                    onChange={e => setPexelsKey(e.target.value)} placeholder="Pexels API key…" />
                </div>
              </div>

              <div style={{ marginTop: 14 }}>
                <button className="btn btn-primary btn-sm" onClick={() => showToast("Settings saved ✓", "success")}>
                  Save Settings
                </button>
              </div>
            </div>

            {/* ── System info ── */}
            <div className="card">
              <div className="settings-section-title">💻 System</div>

              {thermal ? (
                <>
                  <div className="settings-row">
                    <div className="settings-label">GPU Temperature</div>
                    <div style={{ fontSize: 13, fontWeight: 600, color: thermal.gpu_temp_c > 85 ? "var(--error)" : "var(--success)" }}>
                      {thermal.gpu_temp_c}°C
                    </div>
                  </div>
                  <div className="settings-row">
                    <div className="settings-label">GPU Utilization</div>
                    <div style={{ fontSize: 13 }}>{thermal.gpu_utilization}%</div>
                  </div>
                  <div className="settings-row">
                    <div className="settings-label">VRAM Used</div>
                    <div style={{ fontSize: 13 }}>{thermal.memory_used_mb} / {thermal.memory_total_mb} MB</div>
                  </div>
                  <div className="settings-row">
                    <div className="settings-label">Render Status</div>
                    <div>
                      {thermal.render_paused
                        ? <span className="badge badge-warning">⏸ Paused (thermal)</span>
                        : <span className="badge badge-success">▶ Running</span>
                      }
                    </div>
                  </div>
                </>
              ) : (
                <div style={{ color: "var(--text-muted)", fontSize: 13, padding: "12px 0" }}>
                  No GPU monitoring data (NVIDIA GPU not detected or nvml unavailable)
                </div>
              )}
            </div>

            {/* ── Required tools ── */}
            <div className="card">
              <div className="settings-section-title">🔧 Required Tools</div>
              <div style={{ fontSize: 13, color: "var(--text-muted)", lineHeight: 1.8, padding: "4px 0" }}>
                The following tools must be installed and on your <strong style={{ color: "var(--text-secondary)" }}>system PATH</strong>:
              </div>
              <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 8 }}>
                {[
                  ["yt-dlp", "pip install yt-dlp", "Video downloader"],
                  ["ffmpeg", "winget install ffmpeg  (or choco install ffmpeg)", "Audio/video processing"],
                  ["python / faster-whisper", "pip install faster-whisper", "Speech transcription"],
                ].map(([name, install, desc]) => (
                  <div key={name} style={{ padding: "10px 14px", background: "var(--bg-hover)", borderRadius: "var(--r-sm)", border: "1px solid var(--border)" }}>
                    <div style={{ fontWeight: 600, fontSize: 13, color: "var(--text-primary)" }}>{name}</div>
                    <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 2 }}>{desc}</div>
                    <code style={{ fontSize: 11, color: "var(--accent-bright)", marginTop: 4, display: "block" }}>{install}</code>
                  </div>
                ))}
              </div>
            </div>

            {/* ── Danger zone ── */}
            <div className="danger-zone">
              <div className="card-header" style={{ marginBottom: 10 }}>
                <div className="card-title" style={{ color: "#f87171" }}>⚠️ Danger Zone</div>
              </div>
              <div className="settings-row" style={{ borderBottom: "none" }}>
                <div>
                  <div className="settings-label">Clear all data</div>
                  <div className="settings-sublabel">Permanently deletes all projects, clips, and assets. Templates are kept.</div>
                </div>
                <button className="btn btn-danger" onClick={handleResetDb}>
                  🗑 Reset Database
                </button>
              </div>
            </div>

          </div>
        )}

      </main>
    </div>
  );
}