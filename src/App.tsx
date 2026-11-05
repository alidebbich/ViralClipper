import { useState, useEffect, useCallback } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import CreateTab from "./CreateTab";
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
  caption_warning?: string;
  timeline_data?: string;
}

interface EdlEffect {
  t: number;
  dur: number;
  type: string;
  intensity: number;
  reason: string;
}

interface EdlHook {
  options: string[];
  chosen: number;
  strategy: string;
}

interface EdlCaptions {
  style: string;
  words_per_line: number;
  highlight: string;
  font: string;
  position: string;
  rtl: boolean;
  sync_nudge_ms: number;
  keyword_emphasis: string[];
  highlight_color?: string;
}

interface EdlCut {
  start: number;
  end: number;
  reason: string;
}

interface Edl {
  version: number;
  content_type: string;
  language: string;
  style_preset: string;
  why_this_style: string;
  hook: EdlHook;
  effects: EdlEffect[];
  captions: EdlCaptions;
  cuts?: EdlCut[];
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
interface ThumbnailCandidate {
  path: string;
  timestamp: number;
}
interface ThermalStatus {
  gpu_temp_c: number; gpu_utilization: number;
  memory_used_mb: number; memory_total_mb: number;
  throttling: boolean; render_paused: boolean;
}

type Tab = "queue" | "create" | "projects" | "templates" | "settings";

// ── Helpers ────────────────────────────────────────────────────
const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

const STAGE_STEPS = ["Downloading","Transcribing","Analyzing","Scoring","Rendering","Completed"];

const statusBadgeClass = (s: string) => {
  const lower = (s || "").toLowerCase();
  if (lower === "completed" || lower === "rendered") return "badge badge-success";
  if (lower.startsWith("failed") || lower.startsWith("error") || lower.includes("fail")) return "badge badge-error";
  if (["downloading","transcribing","analyzing","scoring","rendering"].includes(lower) || lower.startsWith("rendering"))
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
  const [language, setLanguage] = useState("auto");
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: string } | null>(null);
  const [editingTemplate, setEditingTemplate] = useState<Template | null>(null);

  // Settings state
  const [ollamaUrl, setOllamaUrl] = useState("http://localhost:11434");
  const [geminiKey, setGeminiKey] = useState("");
  const [groqKey, setGroqKey] = useState("");
  const [pexelsKey, setPexelsKey] = useState("");
  const [maxDlTime, setMaxDlTime] = useState(30);
  const [maxTranscribeTime, setMaxTranscribeTime] = useState(60);
  const [autoOpenRender, setAutoOpenRender] = useState(false);
  const [previewClip, setPreviewClip] = useState<Clip | null>(null);
  const [thumbnailCandidates, setThumbnailCandidates] = useState<Record<string, ThumbnailCandidate[]>>({});
  const [selectedThumbs, setSelectedThumbs] = useState<Record<string, string>>({});
  const [zoomedImage, setZoomedImage] = useState<string | null>(null);
  const [editingEdlClip, setEditingEdlClip] = useState<{ clip: Clip; edl: Edl } | null>(null);
  const [isSavingEdl, setIsSavingEdl] = useState(false);

  const toAssetUrl = (filePath: string | undefined | null) => {
    if (!filePath) return undefined;
    const normalized = filePath.replace(/\\/g, "/");
    try {
      return convertFileSrc(normalized);
    } catch {
      return convertFileSrc(filePath);
    }
  };

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

  const loadSettings = useCallback(async () => {
    try {
      const gKey = await invoke<{ value_json: string } | null>("get_user_preference", { key: "gemini_api_key" });
      if (gKey?.value_json) setGeminiKey(gKey.value_json.replace(/^"|"$/g, ''));

      const grqKey = await invoke<{ value_json: string } | null>("get_user_preference", { key: "groq_api_key" });
      if (grqKey?.value_json) setGroqKey(grqKey.value_json.replace(/^"|"$/g, ''));

      const olUrl = await invoke<{ value_json: string } | null>("get_user_preference", { key: "ollama_url" });
      if (olUrl?.value_json) setOllamaUrl(olUrl.value_json.replace(/^"|"$/g, ''));
    } catch {
      // silent
    }
  }, []);

  const handleSaveSettings = async () => {
    try {
      await invoke("set_user_preference", { pref: { id: "gemini_api_key", key: "gemini_api_key", value_json: JSON.stringify(geminiKey.trim()) } });
      await invoke("set_user_preference", { pref: { id: "groq_api_key", key: "groq_api_key", value_json: JSON.stringify(groqKey.trim()) } });
      await invoke("set_user_preference", { pref: { id: "ollama_url", key: "ollama_url", value_json: JSON.stringify(ollamaUrl.trim()) } });
      showToast("Settings saved ✓", "success");
    } catch (e) {
      showToast(`Error saving settings: ${e}`, "error");
    }
  };

  useEffect(() => {
    loadProjects();
    loadTemplates();
    loadAssets();
    loadSettings();

    const unlistenPipeline = listen<PipelineState>("pipeline-update", async (e) => {
      setPipeline(e.payload);
      if (e.payload.current_stage === "Completed") {
        showToast("🎉 Pipeline complete! Taking you to your clips...", "success");
        try {
          const projs = await invoke<Project[]>("get_projects");
          setProjects(projs);
          const p = projs.find(proj => proj.id === e.payload.project_id) || projs[0];
          if (p) {
            setSelectedProject(p);
            const loadedClips = await invoke<Clip[]>("get_clips", { projectId: p.id });
            setClips(loadedClips);
            setTab("projects");
          }
        } catch (err) {
          console.error("Failed to load completed project clips:", err);
          loadProjects();
        }
      } else if (e.payload.current_stage === "Failed") {
        showToast(`Pipeline failed: ${e.payload.error}`, "error");
      }
    });

    const unlistenThermal = listen<ThermalStatus>("thermal-update", (e) => {
      setThermal(e.payload);
    });

    const unlistenClip = listen<{ clip_id: string; status: string; render_path?: string | null; thumbnail_path?: string | null; caption_warning?: string | null }>("clip-update", (e) => {
      setClips(prev => prev.map(c => c.id === e.payload.clip_id ? {
        ...c,
        status: e.payload.status,
        render_path: e.payload.render_path !== undefined ? (e.payload.render_path ?? undefined) : c.render_path,
        thumbnail_path: e.payload.thumbnail_path !== undefined ? (e.payload.thumbnail_path ?? undefined) : c.thumbnail_path,
        caption_warning: e.payload.caption_warning !== undefined ? (e.payload.caption_warning ?? undefined) : c.caption_warning,
      } : c));
    });

    return () => {
      unlistenPipeline.then(fn => fn());
      unlistenThermal.then(fn => fn());
      unlistenClip.then(fn => fn());
    };
  }, [loadProjects, loadTemplates, loadAssets, showToast]);

  const handleStart = async () => {
    if (!urlInput.trim()) { showToast("Paste a YouTube URL first", "error"); return; }
    try {
      setBusy(true);
      await invoke<string>("start_pipeline", {
        request: { source_url: urlInput.trim(), template_id: templateId, max_clips: maxClips, language },
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

  const [renderingAll, setRenderingAll] = useState(false);

  const handleRenderClip = async (clipId: string) => {
    try {
      showToast("Rendering clip...", "info");
      setClips(prev => prev.map(c => c.id === clipId ? { ...c, status: "rendering" } : c));
      const res = await invoke<ClipInfo>("render_clip_cmd", { clipId });
      setClips(prev => prev.map(c => c.id === clipId ? {
        ...c,
        status: "rendered",
        render_path: res.render_path ?? c.render_path
      } : c));
      showToast("Clip rendered successfully!", "success");
    } catch (e) {
      const errStr = String(e);
      setClips(prev => prev.map(c => c.id === clipId ? { ...c, status: `failed: ${errStr}` } : c));
      showToast(`Render failed: ${errStr}`, "error");
    }
  };

  const handleRenderAll = async () => {
    if (!selectedProject) return;
    try {
      setRenderingAll(true);
      showToast("Rendering all clips (max 2 parallel)...", "info");
      await invoke("render_all_clips_cmd", { projectId: selectedProject.id });
      const updatedClips = await invoke<Clip[]>("get_clips", { projectId: selectedProject.id });
      setClips(updatedClips);
      showToast("Render all completed!", "success");
    } catch (e) {
      showToast(`Render all failed: ${e}`, "error");
    } finally {
      setRenderingAll(false);
    }
  };

  const handleOpenFolder = async (path: string) => {
    try {
      await invoke("open_rendered_folder", { path });
    } catch (e) {
      showToast(`Cannot open folder: ${e}`, "error");
    }
  };

  const handleQuickExport = async (clip: Clip) => {
    if (!clip.render_path) return;
    try {
      showToast("Exporting to Downloads\\ViralClipper...", "info");
      const targetPath = await invoke<string>("quick_export_clip", { clipId: clip.id });
      showToast(`⚡ Saved to Downloads\\ViralClipper! Opened in Explorer.`, "success");
    } catch (e) {
      showToast(`Quick export failed: ${e}`, "error");
    }
  };

  const handleExportClip = async (clip: Clip) => {
    if (!clip.render_path) return;
    try {
      const score = clip.viral_score ? `_${clip.viral_score.toFixed(1)}` : "";
      const ts = new Date().toISOString().replace(/[:.]/g, "-").substring(0, 19);
      const cleanName = clip.title.replace(/[\\/:*?"<>|]/g, "_").trim();
      const defaultName = `${cleanName}${score}_${ts}.mp4`;
      let target: string | null = null;
      try {
        target = await save({
          defaultPath: defaultName,
          filters: [{ name: "MP4 Video", extensions: ["mp4"] }]
        });
      } catch (dialogErr) {
        console.warn("Save dialog failed, falling back to quick export:", dialogErr);
        await handleQuickExport(clip);
        return;
      }
      if (target) {
        await invoke("export_rendered_clip", { clipId: clip.id, targetPath: target });
        // Also copy thumbnail next to it if selected
        const thumbPath = selectedThumbs[clip.id] || clip.thumbnail_path;
        if (thumbPath) {
          const thumbTarget = target.replace(/\.mp4$/i, "_thumb.jpg");
          try { await invoke("copy_file", { src: thumbPath, dst: thumbTarget }); } catch {}
        }
        showToast("💾 Saved! Ready for TikTok/Instagram.", "success");
      }
    } catch (e) {
      showToast(`Export failed: ${e}`, "error");
    }
  };

  const handleOpenTikTok = () =>
    invoke("open_url", { url: "https://www.tiktok.com/upload" }).catch(() =>
      window.open("https://www.tiktok.com/upload", "_blank")
    );

  const handleOpenInstagram = () =>
    invoke("open_url", { url: "https://www.instagram.com/create/story/" }).catch(() =>
      window.open("https://www.instagram.com/create/story/", "_blank")
    );

  const handleGenerateThumbnails = async (clip: Clip) => {
    if (!clip.render_path) return;
    try {
      showToast("Generating thumbnail candidates...", "info");
      const candidates = await invoke<ThumbnailCandidate[]>("generate_thumbnails", { clipId: clip.id });
      if (candidates && candidates.length > 0) {
        setThumbnailCandidates(prev => ({ ...prev, [clip.id]: candidates }));
        showToast(`✅ ${candidates.length} thumbnails generated — pick your favourite!`, "success");
      } else {
        showToast("No thumbnails returned", "error");
      }
    } catch (e) {
      showToast(`Thumbnail generation failed: ${e}`, "error");
    }
  };

  const handleDeleteClip = async (clip: Clip) => {
    try {
      await invoke("delete_clip", { clipId: clip.id });
      setClips(prev => prev.filter(c => c.id !== clip.id));
      setPreviewClip(null);
      showToast("Clip deleted", "info");
    } catch (e) {
      showToast(`Delete failed: ${e}`, "error");
    }
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
            ["create", "✨", "Create"],
            ["projects", "🎬", "Projects"],
            ["templates", "🎨", "Templates"],
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
                <select
                  className="template-select-inline"
                  style={{ minWidth: 100 }}
                  value={language}
                  onChange={e => setLanguage(e.target.value)}
                  title="Spoken Language"
                >
                  <option value="auto">🌐 Auto</option>
                  <option value="en">🇺🇸 English</option>
                  <option value="ar">🇸🇦 Arabic</option>
                  <option value="fr">🇫🇷 French</option>
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
                  {pipeline.current_stage === "Completed" && (
                    <button
                      className="btn btn-primary btn-lg"
                      style={{ marginTop: 15, width: "100%" }}
                      onClick={async () => {
                        const projs = await invoke<Project[]>("get_projects");
                        setProjects(projs);
                        const p = projs.find(proj => proj.id === pipeline.project_id) || projs[0];
                        if (p) {
                          setSelectedProject(p);
                          const loadedClips = await invoke<Clip[]>("get_clips", { projectId: p.id });
                          setClips(loadedClips);
                          setTab("projects");
                        }
                      }}
                    >
                      🎬 View All Rendered Clips →
                    </button>
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

        {/* ══ CREATE TAB ══ */}
        {tab === "create" && (
          <CreateTab showToast={showToast} />
        )}

        {/* ══ PROJECTS TAB ══ */}
        {tab === "projects" && (
          <div className="clips-view">
            {selectedProject ? (
              <>
                <div className="back-row" style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <button className="btn btn-ghost btn-sm" onClick={() => { setSelectedProject(null); setClips([]); }}>
                    ← Back
                  </button>
                  <div>
                    <h2>{selectedProject.title}</h2>
                    <div style={{ fontSize: 12, color: "var(--text-muted)", marginTop: 2 }}>{selectedProject.source_url}</div>
                  </div>
                  <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 8 }}>
                    <button
                      className="btn btn-primary btn-sm"
                      disabled={renderingAll}
                      onClick={handleRenderAll}
                      title="Render all clips in parallel (max 2)"
                    >
                      {renderingAll ? "⏳ Rendering All..." : "🎬 Render All Clips"}
                    </button>
                    <span className={statusBadgeClass(selectedProject.status)}>
                      {selectedProject.status}
                    </span>
                  </div>
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
                          {clip.render_path ? (
                            <div className="clip-video-wrapper" style={{ width: "100%", height: "100%", position: "relative" }}>
                              <video
                                src={toAssetUrl(clip.render_path)}
                                poster={toAssetUrl(clip.thumbnail_path)}
                                controls
                                playsInline
                                preload="metadata"
                                style={{ width: "100%", height: "100%", objectFit: "cover" }}
                              />
                            </div>
                          ) : (
                            <div className="clip-thumb-placeholder">
                              <span className="clip-placeholder-icon">🎬</span>
                              <span className="clip-placeholder-text">
                                {clip.status === "rendering" ? "Rendering 9:16 clip..." : "Not rendered yet"}
                              </span>
                            </div>
                          )}
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
                            <span>⏳ {fmt(clip.duration_s)}</span>
                            {clip.hook_score != null && (
                              <span title="AI estimated hook performance based on opening 3 seconds">
                                🎯 Hook: {(clip.hook_score * 100).toFixed(0)}% (AI estimate)
                              </span>
                            )}
                          </div>

                          {clip.status.toLowerCase().startsWith("failed") && (
                            <div className="clip-error-box">
                              ⚠️ {clip.status.replace(/^failed:\s*/i, '')}
                            </div>
                          )}

                          {clip.caption_warning && (
                            <div className="clip-warning-box" style={{
                              background: "rgba(234, 179, 8, 0.15)",
                              border: "1px solid rgba(234, 179, 8, 0.4)",
                              color: "#facc15",
                              padding: "6px 10px",
                              borderRadius: "6px",
                              fontSize: "12px",
                              marginTop: "8px",
                              display: "flex",
                              alignItems: "center",
                              gap: "6px"
                            }}>
                              <span>⚠️</span>
                              <span>{clip.caption_warning}</span>
                            </div>
                          )}

                          {clip.llm_reason && <div className="clip-reason">{clip.llm_reason}</div>}

                          {(() => {
                            let edl: Edl | null = null;
                            if (clip.timeline_data) {
                              try { edl = JSON.parse(clip.timeline_data); } catch {}
                            } else if (clip.heuristic_json) {
                              try { edl = JSON.parse(clip.heuristic_json); } catch {}
                            }
                            const styleName = edl?.style_preset || clip.template_id || "viral_high_energy";
                            const styleEmoji = styleName.includes("podcast") ? "🎙️ Podcast Clean" : styleName.includes("cinematic") ? "🎬 Cinematic" : styleName.includes("explainer") ? "💡 Explainer" : "🔥 Viral High-Energy";
                            const punchCount = edl?.effects?.filter((e: any) => e.type === "punch_in").length || 0;
                            const keywordCount = edl?.captions?.keyword_emphasis?.length || 0;
                            const cutsCount = edl?.cuts?.length || 0;
                            const totalCutSecs = edl?.cuts?.reduce((acc: number, c: any) => acc + (c.end - c.start), 0) || 0;

                            return (
                              <div className="edl-badge-row">
                                <span className="edl-pill" title="Creative Style Preset">{styleEmoji}</span>
                                {punchCount > 0 && (
                                  <span className="edl-pill zooms" title="Story-aware Camera Punch-ins">
                                    ⚡ {punchCount} Story Zoom{punchCount > 1 ? "s" : ""}
                                  </span>
                                )}
                                {cutsCount > 0 && (
                                  <span className="edl-pill cuts" title={`Dead-air trimmer removed ${cutsCount} pause${cutsCount > 1 ? "s" : ""} (${totalCutSecs.toFixed(1)}s)`}>
                                    ✂️ {cutsCount} Cut{cutsCount > 1 ? "s" : ""} (-{totalCutSecs.toFixed(1)}s)
                                  </span>
                                )}
                                {keywordCount > 0 && (
                                  <span className="edl-pill keywords" title="Keywords emphasized in captions">
                                    🎯 {keywordCount} Keyword{keywordCount > 1 ? "s" : ""}
                                  </span>
                                )}
                                <button
                                  className="btn btn-ghost btn-sm"
                                  style={{ padding: "2px 8px", fontSize: 11, marginLeft: "auto", border: "1px solid rgba(255,255,255,0.15)" }}
                                  onClick={async (e) => {
                                    e.stopPropagation();
                                    try {
                                      const loaded = await invoke<Edl>("get_clip_edl", { clipId: clip.id });
                                      setEditingEdlClip({ clip, edl: loaded });
                                    } catch (err: any) {
                                      showToast("Failed to load edit plan: " + err, "error");
                                    }
                                  }}
                                  title="Inspect & edit creative decisions (EDL)"
                                >
                                  ✏️ Edit Plan
                                </button>
                              </div>
                            );
                          })()}

                          {/* Thumbnail picker (shown after generation) */}
                          {thumbnailCandidates[clip.id] && thumbnailCandidates[clip.id].length > 0 && (
                            <div className="thumb-picker">
                              <div className="thumb-picker-label">🖼 Pick Thumbnail <span style={{color:"var(--accent-bright)",marginLeft:8}}>⭐ = Gemini AI Pick</span></div>
                              <div className="thumb-picker-grid">
                                {thumbnailCandidates[clip.id].map((t, i) => (
                                  <div key={i} className="thumb-option-wrap">
                                    <img
                                      src={toAssetUrl(t.path)}
                                      className={`thumb-option ${selectedThumbs[clip.id] === t.path ? "thumb-selected" : ""}`}
                                      onClick={() => setZoomedImage(t.path)}
                                      alt={`Thumbnail at ${t.timestamp.toFixed(0)}s`}
                                      title="Click to enlarge"
                                    />
                                    {i === 0 && (
                                      <div className="thumb-ai-badge">⭐ AI Pick</div>
                                    )}
                                    <button
                                      className={`btn btn-sm ${selectedThumbs[clip.id] === t.path ? "btn-primary" : "btn-secondary"}`}
                                      onClick={() => setSelectedThumbs(prev => ({ ...prev, [clip.id]: t.path }))}
                                      style={{ width: "100%", marginTop: "8px", fontSize: "12px", padding: "4px 8px" }}
                                    >
                                      {selectedThumbs[clip.id] === t.path ? "✓ Selected" : "Select This Thumbnail"}
                                    </button>
                                  </div>
                                ))}
                              </div>
                              {selectedThumbs[clip.id] && (
                                <div className="thumb-selected-label">✓ Thumbnail selected — will be saved alongside your exported clip</div>
                              )}
                            </div>
                          )}

                          <div className="clip-actions" style={{ marginTop: 10, display: "flex", flexWrap: "wrap", gap: 6, alignItems: "center" }}>
                            {clip.render_path ? (
                              <>
                                <button
                                  className="btn btn-primary btn-sm"
                                  onClick={() => setPreviewClip(clip)}
                                  title="Watch full vertical video in app"
                                >
                                  ▶ Watch
                                </button>
                                <button
                                  className="btn btn-sm"
                                  style={{ background: "#10b981", color: "#fff", border: "none", fontWeight: 600 }}
                                  onClick={() => handleQuickExport(clip)}
                                  title="Instant export to Downloads\ViralClipper & reveal in Explorer"
                                >
                                  ⚡ Quick Export
                                </button>
                                <button
                                  className="btn btn-secondary btn-sm"
                                  onClick={() => handleExportClip(clip)}
                                  title="Save As to custom location"
                                >
                                  💾 Save As...
                                </button>
                                <button
                                  className="btn btn-secondary btn-sm"
                                  onClick={handleOpenTikTok}
                                  title="Open TikTok upload page"
                                >
                                  TikTok ↗
                                </button>
                                <button
                                  className="btn btn-secondary btn-sm"
                                  onClick={handleOpenInstagram}
                                  title="Open Instagram Reels upload"
                                >
                                  IG Reels ↗
                                </button>
                                <button
                                  className="btn btn-ghost btn-sm"
                                  onClick={() => handleOpenFolder(clip.render_path!)}
                                >
                                  📂
                                </button>
                                <button
                                  className="btn btn-ghost btn-sm"
                                  onClick={() => handleRenderClip(clip.id)}
                                  title="Re-render"
                                >
                                  🔄
                                </button>
                              </>
                            ) : clip.status === "rendering" ? (
                              <button className="btn btn-ghost btn-sm" disabled>
                                ⏳ Rendering...
                              </button>
                            ) : (
                              <button
                                className="btn btn-primary btn-sm"
                                onClick={() => handleRenderClip(clip.id)}
                                title="Render this clip to 9:16 vertical video with captions"
                              >
                                {clip.status.toLowerCase().startsWith("failed") ? "🔁 Retry Render" : "⚡ Render"}
                              </button>
                            )}
                            <span className={statusBadgeClass(clip.status)} style={{ marginLeft: "auto" }}>
                              {clip.status.toLowerCase().startsWith("failed") ? "FAILED" : clip.status.toUpperCase()}
                            </span>
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
            <div style={{ marginBottom: 20 }}>
              <h2 style={{ fontFamily: "var(--font-display)", fontSize: 20, fontWeight: 700 }}>Templates</h2>
              <p style={{ fontSize: 13, color: "var(--text-muted)", marginTop: 4 }}>
                Choose a style template. Controls caption style, colours, and visual effects applied to your clips.
              </p>
            </div>
            <div className="templates-grid">
              {templates.map(t => {
                // Extract primary color from theme_json for accent stripe
                let primaryColor = "#8b5cf6";
                try {
                  const theme = JSON.parse(t.theme_json);
                  if (theme.primary) primaryColor = theme.primary;
                } catch {}
                const isSelected = templateId === t.id;
                const emoji = t.id === "tiktok" ? "🔥" : t.id === "hormozi" ? "⚡" : t.id === "cinematic" ? "🎬" : t.id === "minimal" ? "✨" : "🎨";
                return (
                  <div
                    key={t.id}
                    className={`template-card-v2 ${isSelected ? "selected" : ""}`}
                    style={{
                      borderLeftColor: primaryColor,
                      boxShadow: isSelected ? `0 0 0 2px ${primaryColor}` : undefined,
                    }}
                    onClick={() => { setTemplateId(t.id); showToast(`Selected: ${t.name}`, "success"); }}
                  >
                    <div className="tmpl-accent" style={{ background: primaryColor }} />
                    <div className="tmpl-body">
                      <div className="tmpl-row">
                        <span className="tmpl-emoji">{emoji}</span>
                        <span className="tmpl-name">{t.name}</span>
                        {isSelected && (
                          <div style={{ marginLeft: "auto", display: "flex", gap: 6, alignItems: "center" }}>
                            <button
                              className="btn btn-ghost btn-sm"
                              style={{ padding: "2px 6px", fontSize: 11 }}
                              onClick={(e) => { e.stopPropagation(); setEditingTemplate(t); }}
                            >
                              ✏️ Edit Style
                            </button>
                            <span className="tmpl-check">✓</span>
                          </div>
                        )}
                      </div>
                      <div className="tmpl-palette">
                        {(() => {
                          let colors = [primaryColor, "#ffffff", "#000000"];
                          try {
                            const th = JSON.parse(t.theme_json);
                            colors = [th.primary || primaryColor, th.secondary || "#ffffff", th.accent || "#000000"];
                          } catch {}
                          return colors.map((c, i) => (
                            <span key={i} className="tmpl-swatch" style={{ background: c }} title={c} />
                          ));
                        })()}
                      </div>
                      {t.description && <div className="tmpl-desc">{t.description}</div>}
                      <div className="tmpl-style-label">
                        {t.id === "tiktok" ? "Viral Bold • Pop-In Highlight"
                          : t.id === "hormozi" ? "Hormozi • Punch Zoom & Yellow"
                          : t.id === "cinematic" ? "Cinematic • Sharp Framed Ambient"
                          : t.id === "minimal" ? "Minimal • Subtle Fade"
                          : "Custom Edit Style"}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Assets tab removed — B-roll library is a future feature */}

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
                  <div className="settings-label">Groq API Key</div>
                  <div className="settings-sublabel">Primary transcription — fast &amp; free (console.groq.com)</div>
                </div>
                <div className="settings-control">
                  <input className="form-input" type="password" value={groqKey}
                    onChange={e => setGroqKey(e.target.value)} placeholder="gsk_…" />
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
                <button className="btn btn-primary btn-sm" onClick={handleSaveSettings}>
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

      {/* ══ VIDEO PREVIEW MODAL ══ */}
      {previewClip && (
        <div
          className="modal-overlay"
          onClick={(e) => { if (e.target === e.currentTarget) setPreviewClip(null); }}
          onKeyDown={(e) => e.key === "Escape" && setPreviewClip(null)}
          tabIndex={-1}
        >
          <div className="modal-container">
            <div className="modal-header">
              <span className="modal-title">{previewClip.title}</span>
              <button className="modal-close" onClick={() => setPreviewClip(null)}>✕</button>
            </div>
            <div className="modal-video-wrap">
              <video
                key={previewClip.id}
                src={toAssetUrl(previewClip.render_path)}
                controls
                autoPlay
                playsInline
                className="modal-video"
              />
            </div>
            <div className="modal-actions" style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
              <button
                className="btn btn-primary"
                onClick={() => { handleGenerateThumbnails(previewClip); setPreviewClip(null); }}
              >
                ✅ Accept &amp; Generate Thumbnails
              </button>
              <button
                className="btn"
                style={{ background: "#10b981", color: "#fff", border: "none", fontWeight: 600 }}
                onClick={() => handleQuickExport(previewClip)}
              >
                ⚡ Quick Export (Downloads)
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => handleExportClip(previewClip)}
              >
                💾 Save As...
              </button>
              <button
                className="btn btn-danger"
                onClick={() => handleDeleteClip(previewClip)}
              >
                🗑 Delete Clip
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ══ ZOOMED IMAGE MODAL ══ */}
      {zoomedImage && (
        <div
          className="modal-overlay"
          onClick={() => setZoomedImage(null)}
          style={{ zIndex: 9999, backdropFilter: "blur(5px)" }}
        >
          <div className="modal-container" style={{ background: "transparent", border: "none", boxShadow: "none", width: "auto", maxWidth: "90vw" }}>
            <img 
              src={toAssetUrl(zoomedImage)} 
              alt="Zoomed Thumbnail" 
              style={{ maxHeight: "90vh", maxWidth: "100%", objectFit: "contain", borderRadius: "12px", boxShadow: "0 10px 40px rgba(0,0,0,0.8)" }} 
            />
          </div>
        </div>
      )}

      {/* ══ TEMPLATE EDIT MODAL ══ */}
      {editingTemplate && (
        <div
          className="modal-overlay"
          onClick={(e) => { if (e.target === e.currentTarget) setEditingTemplate(null); }}
        >
          <div className="modal-container" style={{ maxWidth: 400 }}>
            <div className="modal-header">
              <span className="modal-title">Edit Caption Style ({editingTemplate.name})</span>
              <button className="modal-close" onClick={() => setEditingTemplate(null)}>✕</button>
            </div>
            <div style={{ padding: 20 }}>
              {(() => {
                let style = { font: "Arial", font_size: 64, primary_color: "&HFFFFFF", secondary_color: "&H00FFFF", outline_color: "&H000000" };
                try { style = { ...style, ...JSON.parse(editingTemplate.ass_style_json) }; } catch {}
                
                // Helper to convert ASS color (&HBBGGRR) to Hex (#RRGGBB)
                const assToHex = (c: string) => {
                  if (c.startsWith("&H") && c.length >= 8) {
                    return "#" + c.substring(6,8) + c.substring(4,6) + c.substring(2,4);
                  }
                  return "#ffffff";
                };
                const hexToAss = (h: string) => {
                  if (h.startsWith("#") && h.length === 7) {
                    return "&H" + h.substring(5,7) + h.substring(3,5) + h.substring(1,3);
                  }
                  return "&HFFFFFF";
                };

                return (
                  <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                    <div className="settings-row" style={{ padding: 0, border: "none" }}>
                      <div className="settings-label">Font Size</div>
                      <input type="number" className="form-input" style={{ width: 100 }} value={style.font_size} onChange={e => {
                        const newStyle = { ...style, font_size: +e.target.value };
                        setEditingTemplate({ ...editingTemplate, ass_style_json: JSON.stringify(newStyle) });
                      }} />
                    </div>
                    <div className="settings-row" style={{ padding: 0, border: "none" }}>
                      <div className="settings-label">Primary Color</div>
                      <input type="color" value={assToHex(style.primary_color)} onChange={e => {
                        const newStyle = { ...style, primary_color: hexToAss(e.target.value) };
                        setEditingTemplate({ ...editingTemplate, ass_style_json: JSON.stringify(newStyle) });
                      }} />
                    </div>
                    <div className="settings-row" style={{ padding: 0, border: "none" }}>
                      <div className="settings-label">Secondary Color (Highlight)</div>
                      <input type="color" value={assToHex(style.secondary_color)} onChange={e => {
                        const newStyle = { ...style, secondary_color: hexToAss(e.target.value) };
                        setEditingTemplate({ ...editingTemplate, ass_style_json: JSON.stringify(newStyle) });
                      }} />
                    </div>
                    <button className="btn btn-primary" style={{ marginTop: 10 }} onClick={async () => {
                      try {
                        await invoke("save_template", { template: editingTemplate });
                        const updated = await invoke<Template[]>("list_templates");
                        setTemplates(updated);
                        setEditingTemplate(null);
                        showToast("Template style saved!", "success");
                      } catch (e: any) {
                        showToast("Error saving template: " + e, "error");
                      }
                    }}>Save Changes</button>
                  </div>
                );
              })()}
            </div>
          </div>
        </div>
      )}
      {/* ══ CREATIVE DECISION STUDIO (EDL) MODAL ══ */}
      {editingEdlClip && (
        <div
          className="modal-overlay"
          onClick={(e) => { if (e.target === e.currentTarget && !isSavingEdl) setEditingEdlClip(null); }}
        >
          <div className="modal-container" style={{ maxWidth: 540, maxHeight: "90vh", overflowY: "auto" }}>
            <div className="modal-header">
              <span className="modal-title">Creative Edit Decision Plan</span>
              <button className="modal-close" onClick={() => setEditingEdlClip(null)}>✕</button>
            </div>
            <div style={{ padding: 20 }}>
              <div style={{ marginBottom: 16 }}>
                <div style={{ fontSize: 14, fontWeight: 700, color: "var(--text-primary)" }}>{editingEdlClip.clip.title}</div>
                <div style={{ fontSize: 12, color: "var(--text-muted)", marginTop: 2 }}>
                  {editingEdlClip.clip.start_s.toFixed(1)}s – {editingEdlClip.clip.end_s.toFixed(1)}s ({editingEdlClip.clip.duration_s.toFixed(1)}s duration)
                </div>
              </div>

              {/* Rationale explanation */}
              <div className="edl-decision-box" style={{ marginBottom: 16, borderLeft: "3px solid var(--accent)" }}>
                <div style={{ fontWeight: 600, color: "var(--accent-bright)", marginBottom: 4 }}>🧠 Editor Rationale</div>
                <div style={{ color: "var(--text-secondary)" }}>{editingEdlClip.edl.why_this_style || "Story-aware pacing with dynamic hook retention."}</div>
              </div>

              {/* Creative Style Preset */}
              <div className="edl-field-group">
                <label>Creative Style Preset</label>
                <select
                  className="form-input"
                  value={editingEdlClip.edl.style_preset}
                  onChange={(e) => {
                    const preset = e.target.value;
                    setEditingEdlClip({
                      ...editingEdlClip,
                      edl: {
                        ...editingEdlClip.edl,
                        style_preset: preset,
                        captions: {
                          ...editingEdlClip.edl.captions,
                          highlight_color: preset === "podcast_clean" ? "#00F0FF" : preset === "cinematic_story" ? "#E2B714" : preset === "educational_explainer" ? "#00FF66" : "#FFE600",
                        }
                      }
                    });
                  }}
                >
                  <option value="viral_high_energy">🔥 High-Energy Viral (Hook Punch, Pop Karaoke, Kinetic Emphasis)</option>
                  <option value="podcast_clean">🎙️ Clean Podcast (Subtle Framing, Clean Captions, Calm Pacing)</option>
                  <option value="cinematic_story">🎬 Cinematic Story (Ambient Blur, Dramatic Push-in, Gold Typography)</option>
                  <option value="educational_explainer">💡 Educational Explainer (High Contrast, Green Keyword Focus, Steady)</option>
                </select>
              </div>

              {/* Hook Text Selection */}
              <div className="edl-field-group">
                <label>Opening Hook Banner</label>
                <input
                  type="text"
                  className="form-input"
                  value={editingEdlClip.edl.hook.options[editingEdlClip.edl.hook.chosen] || ""}
                  onChange={(e) => {
                    const newOptions = [...editingEdlClip.edl.hook.options];
                    newOptions[editingEdlClip.edl.hook.chosen] = e.target.value;
                    setEditingEdlClip({
                      ...editingEdlClip,
                      edl: {
                        ...editingEdlClip.edl,
                        hook: { ...editingEdlClip.edl.hook, options: newOptions }
                      }
                    });
                  }}
                  placeholder="Enter hook headline..."
                />
                {editingEdlClip.edl.hook.options.length > 1 && (
                  <div style={{ display: "flex", gap: 6, marginTop: 4, flexWrap: "wrap" }}>
                    {editingEdlClip.edl.hook.options.map((opt, idx) => (
                      <button
                        key={idx}
                        className={`btn btn-sm ${editingEdlClip.edl.hook.chosen === idx ? "btn-primary" : "btn-ghost"}`}
                        style={{ fontSize: 11, padding: "2px 8px" }}
                        onClick={() => {
                          setEditingEdlClip({
                            ...editingEdlClip,
                            edl: {
                              ...editingEdlClip.edl,
                              hook: { ...editingEdlClip.edl.hook, chosen: idx }
                            }
                          });
                        }}
                      >
                        Option {idx + 1}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              {/* Story Camera Movement */}
              <div className="edl-field-group">
                <label style={{ display: "flex", justifyContent: "space-between" }}>
                  <span>Camera Movement Keyframes ({editingEdlClip.edl.effects.length})</span>
                  <span style={{ color: "var(--accent-bright)", fontSize: 11 }}>Punch Intensity: {((editingEdlClip.edl.effects[0]?.intensity || 0.14) * 100).toFixed(0)}%</span>
                </label>
                <input
                  type="range"
                  min="0.06"
                  max="0.25"
                  step="0.02"
                  value={editingEdlClip.edl.effects[0]?.intensity || 0.14}
                  onChange={(e) => {
                    const intensity = parseFloat(e.target.value);
                    const newEffects = editingEdlClip.edl.effects.map(eff => ({ ...eff, intensity }));
                    setEditingEdlClip({
                      ...editingEdlClip,
                      edl: { ...editingEdlClip.edl, effects: newEffects }
                    });
                  }}
                />
                {editingEdlClip.edl.effects.map((eff, i) => (
                  <div key={i} className="edl-effect-item">
                    <span>⚡ {eff.t.toFixed(1)}s – {(eff.t + eff.dur).toFixed(1)}s: <strong>{eff.type}</strong></span>
                    <span style={{ color: "var(--text-muted)", fontSize: 11 }}>{eff.reason}</span>
                  </div>
                ))}
              </div>

              {/* Keyword Emphasis List */}
              <div className="edl-field-group">
                <label>Emphasized Keywords (Contrasting Color & Pop)</label>
                <input
                  type="text"
                  className="form-input"
                  value={editingEdlClip.edl.captions.keyword_emphasis.join(", ")}
                  onChange={(e) => {
                    const words = e.target.value.split(",").map(w => w.trim().toLowerCase()).filter(Boolean);
                    setEditingEdlClip({
                      ...editingEdlClip,
                      edl: {
                        ...editingEdlClip.edl,
                        captions: { ...editingEdlClip.edl.captions, keyword_emphasis: words }
                      }
                    });
                  }}
                  placeholder="e.g. secret, viral, mistake, money"
                />
              </div>

              {/* Speech Pacing & Dead-Air Cuts */}
              {editingEdlClip.edl.cuts && editingEdlClip.edl.cuts.length > 0 && (
                <div className="edl-field-group">
                  <label style={{ display: "flex", justifyContent: "space-between" }}>
                    <span>Speech Pacing Trimmer ({editingEdlClip.edl.cuts.length} Jump Cuts)</span>
                    <span style={{ color: "var(--accent-bright)", fontSize: 11 }}>
                      Saved: {editingEdlClip.edl.cuts.reduce((a, c) => a + (c.end - c.start), 0).toFixed(1)}s dead air
                    </span>
                  </label>
                  <div style={{ maxHeight: 110, overflowY: "auto", border: "1px solid rgba(255,255,255,0.06)", borderRadius: 6, padding: "4px 8px", background: "rgba(0,0,0,0.2)" }}>
                    {editingEdlClip.edl.cuts.map((c, i) => (
                      <div key={i} className="edl-effect-item" style={{ marginTop: 3, marginBottom: 3, padding: "4px 8px" }}>
                        <span>✂️ Cut {c.start.toFixed(2)}s – {c.end.toFixed(2)}s <span style={{ color: "var(--text-muted)", fontSize: 11 }}>(-{(c.end - c.start).toFixed(2)}s)</span></span>
                        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>{c.reason}</span>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Actions */}
              <div style={{ display: "flex", gap: 10, marginTop: 20 }}>
                <button
                  className="btn btn-primary"
                  style={{ flex: 1 }}
                  disabled={isSavingEdl}
                  onClick={async () => {
                    try {
                      setIsSavingEdl(true);
                      showToast("Saving updated edit plan...", "info");
                      await invoke("update_clip_edl", {
                        clipId: editingEdlClip.clip.id,
                        edl: editingEdlClip.edl,
                      });
                      if (selectedProject) {
                        const updatedClips = await invoke<Clip[]>("get_clips", { projectId: selectedProject.id });
                        setClips(updatedClips);
                      }
                      showToast("✅ Edit Decision Plan saved!", "success");
                      setEditingEdlClip(null);
                    } catch (e: any) {
                      showToast("Failed to save plan: " + e, "error");
                    } finally {
                      setIsSavingEdl(false);
                    }
                  }}
                >
                  Save Edit Plan
                </button>
                <button
                  className="btn btn-secondary"
                  disabled={isSavingEdl}
                  onClick={async () => {
                    try {
                      setIsSavingEdl(true);
                      showToast("Saving and re-rendering with new plan...", "info");
                      await invoke("update_clip_edl", {
                        clipId: editingEdlClip.clip.id,
                        edl: editingEdlClip.edl,
                      });
                      const clipId = editingEdlClip.clip.id;
                      setEditingEdlClip(null);
                      handleRenderClip(clipId);
                    } catch (e: any) {
                      showToast("Failed to re-render: " + e, "error");
                    } finally {
                      setIsSavingEdl(false);
                    }
                  }}
                >
                  ⚡ Save & Re-render
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}