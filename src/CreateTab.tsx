import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function CreateTab({ showToast }: { showToast: (msg: string, type: string) => void }) {
  const [script, setScript] = useState("");
  const [language, setLanguage] = useState("auto");
  const [loading, setLoading] = useState(false);
  const [voices, setVoices] = useState<string[]>([]);
  const [selectedVoice, setSelectedVoice] = useState("");
  const [plan, setPlan] = useState<any>(null);

  useEffect(() => {
    invoke<string[]>("list_tts_voices_cmd")
      .then(v => {
        setVoices(v);
        if (v.length > 0) setSelectedVoice(v[0]);
      })
      .catch(e => showToast("Failed to load voices: " + e, "error"));
  }, []);

  const handleGenerate = async () => {
    if (!script.trim()) return showToast("Enter a script first", "error");
    setLoading(true);
    try {
      showToast("Generating scenes with AI...", "success");
      const res = await invoke("generate_script_plan_cmd", {
        rawScript: script,
        language: language
      });
      setPlan(res);
      showToast("Scenes generated!", "success");
    } catch (e: any) {
      showToast("Error: " + e, "error");
    }
    setLoading(false);
  };

  return (
    <div className="tab-content create-tab">
      <div className="settings-section">
        <h2 className="section-title">✨ Script to Video</h2>
        
        {!plan ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <textarea
              className="form-input"
              style={{ minHeight: 200, resize: "vertical" }}
              placeholder="Paste your script here..."
              value={script}
              onChange={e => setScript(e.target.value)}
            />
            
            <div style={{ display: "flex", gap: 16 }}>
              <div className="settings-row" style={{ flex: 1, border: "none", padding: 0 }}>
                <div className="settings-label">Language</div>
                <select className="form-input" value={language} onChange={e => setLanguage(e.target.value)}>
                  <option value="auto">Auto-Detect</option>
                  <option value="en">English</option>
                  <option value="ar">Arabic</option>
                  <option value="fr">French</option>
                </select>
              </div>
              <div className="settings-row" style={{ flex: 1, border: "none", padding: 0 }}>
                <div className="settings-label">Voice</div>
                <select className="form-input" value={selectedVoice} onChange={e => setSelectedVoice(e.target.value)}>
                  {voices.map(v => <option key={v} value={v}>{v}</option>)}
                </select>
              </div>
            </div>

            <button 
              className="btn btn-primary" 
              onClick={handleGenerate}
              disabled={loading}
              style={{ alignSelf: "flex-start", marginTop: 10, padding: "10px 24px" }}
            >
              {loading ? "Generating..." : "Generate Scenes"}
            </button>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <h3 style={{ margin: 0, color: "var(--text)" }}>Timeline</h3>
              <button className="btn btn-secondary btn-sm" onClick={() => setPlan(null)}>
                ← Back to Edit
              </button>
            </div>
            
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              {plan.scenes.map((scene: any, i: number) => (
                <div key={i} className="clip-card" style={{ padding: 16 }}>
                  <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 8 }}>
                    <div className="clip-title">Scene {i + 1}</div>
                    <div className="badge badge-pending">~{scene.duration_hint?.toFixed(1)}s</div>
                  </div>
                  <div style={{ color: "var(--text-muted)", fontSize: 13, marginBottom: 8, fontStyle: "italic" }}>
                    "{scene.text}"
                  </div>
                  <div style={{ display: "flex", gap: 8, flexWrap: "wrap", fontSize: 12 }}>
                    <div className="badge badge-success">📷 {scene.visual_type}</div>
                    <div className="badge badge-processing">🎥 {scene.camera}</div>
                    <div className="badge badge-processing">🎭 {scene.mood}</div>
                  </div>
                  <div style={{ marginTop: 8, color: "var(--text-muted)", fontSize: 12 }}>
                    Keywords: {scene.visual_keywords?.join(", ")}
                  </div>
                  <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
                    <button className="btn btn-secondary btn-sm">▶ Preview Voice</button>
                    <button className="btn btn-secondary btn-sm" disabled>Swap Visual (Coming Soon)</button>
                  </div>
                </div>
              ))}
            </div>
            
            <button 
              className="btn btn-primary" 
              style={{ marginTop: 16, padding: "12px" }}
              disabled={loading}
              onClick={async () => {
                setLoading(true);
                showToast("Starting assembly...", "success");
                try {
                  const finalPath = await invoke<string>("assemble_video_cmd", { plan });
                  showToast("Video rendered at: " + finalPath, "success");
                } catch (e: any) {
                  showToast("Error rendering: " + e, "error");
                }
                setLoading(false);
              }}
            >
              {loading ? "Rendering (This may take a while)..." : "Render Full Video (Mock Visuals)"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
