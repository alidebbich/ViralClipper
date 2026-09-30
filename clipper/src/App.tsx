import { useState, useCallback } from 'react'
import { 
  Download, 
  Mic, 
  Scissors, 
  Zap, 
  Trophy, 
  Settings,
  Plus,
  X,
  Loader2,
  CheckCircle,
  AlertTriangle,
  Clock,
  Image,
  Layers,
  Trash2,
  Play,
  FolderOpen,
  Copy,
  Share2,
  ChevronRight,
  Sparkles,
  Brain,
  Film,
  Wand2,
  Maximize2,
  Globe,
  Hash,
  Zap as ZapIcon,
  RefreshCw,
  Check
} from 'lucide-react'
import { cn } from './lib/utils'

// Types
interface Project {
  id: string
  url: string
  title: string
  status: 'pending' | 'downloading' | 'transcribing' | 'analyzing' | 'rendering' | 'completed' | 'error'
  progress: number
  stage: string
  clips?: Clip[]
  createdAt: Date
  error?: string
  thumbnail?: string
}

interface Clip {
  id: string
  start: number
  end: number
  duration: number
  title: string
  hook: string
  viralScore: number
  heuristicScore: number
  reason: string
  breakdown: Record<string, number>
  text: string
  status: 'pending' | 'rendering' | 'completed' | 'error'
  renderPath?: string
  thumbnail?: string
}

interface Template {
  id: string
  name: string
  description: string
  preview: string
}

// Status config
const STAGE_CONFIG: Record<Project['status'], { label: string; icon: React.ReactNode; color: string; gradient: string }> = {
  pending: { 
    label: 'Queued', 
    icon: <Clock className="w-4 h-4" />, 
    color: 'text-[var(--fg-subtle)]',
    gradient: 'from-[var(--fg-subtle)] to-[var(--fg-subtle)]'
  },
  downloading: { 
    label: 'Downloading', 
    icon: <Loader2 className="w-4 h-4 animate-spin" />, 
    color: 'text-[var(--primary)]',
    gradient: 'from-[var(--primary)] to-[var(--primary-dim)]'
  },
  transcribing: { 
    label: 'Transcribing', 
    icon: <Mic className="w-4 h-4 animate-pulse" />, 
    color: 'text-blue-400',
    gradient: 'from-blue-400 to-blue-600'
  },
  analyzing: { 
    label: 'AI Analyzing', 
    icon: <Brain className="w-4 h-4 animate-pulse" />, 
    color: 'text-purple-400',
    gradient: 'from-purple-400 to-purple-600'
  },
  rendering: { 
    label: 'Rendering', 
    icon: <Film className="w-4 h-4 animate-pulse" />, 
    color: 'text-orange-400',
    gradient: 'from-orange-400 to-orange-600'
  },
  completed: { 
    label: 'Complete', 
    icon: <CheckCircle className="w-4 h-4" />, 
    color: 'text-[var(--success)]',
    gradient: 'from-[var(--success)] to-emerald-600'
  },
  error: { 
    label: 'Error', 
    icon: <AlertTriangle className="w-4 h-4" />, 
    color: 'text-[var(--danger)]',
    gradient: 'from-[var(--danger)] to-red-600'
  },
}

const TEMPLATES: Template[] = [
  { id: 'tiktok', name: 'TikTok Style', description: 'Pop-in karaoke captions, neon accents, fast pacing', preview: '🎵' },
  { id: 'hormozi', name: 'Hormozi Style', description: 'Bold yellow highlights, high contrast, authority framing', preview: '💼' },
  { id: 'minimal', name: 'Minimal Clean', description: 'Clean white captions, subtle progress bar, distraction-free', preview: '✨' },
]

// Utility components
function StatusBadge({ status, compact = false }: { status: Project['status']; compact?: boolean }) {
  const config = STAGE_CONFIG[status]
  const isActive = ['downloading', 'transcribing', 'analyzing', 'rendering'].includes(status)
  
  return (
    <span className={cn(
      'inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-all duration-300',
      compact ? 'px-2.5 py-0.5' : '',
      status === 'completed' && 'bg-[var(--success)]/15 text-[var(--success)] border border-[var(--success)]/30',
      status === 'error' && 'bg-[var(--danger)]/15 text-[var(--danger)] border border-[var(--danger)]/30',
      status === 'pending' && 'bg-[var(--bg-elevated)] text-[var(--fg-muted)] border border-[var(--border)]',
      ['downloading', 'transcribing', 'analyzing', 'rendering'].includes(status) && 
        'bg-gradient-to-r bg-[var(--primary)]/15 from-[var(--primary)]/20 to-[var(--primary-dim)]/20 text-[var(--primary)] border border-[var(--primary)]/30 animate-pulse-glow'
    )}>
      <span className={cn('flex items-center', config.color)}>
        {config.icon}
      </span>
      {config.label}
      {isActive && !compact && (
        <span className="w-1.5 h-1.5 rounded-full bg-[var(--primary)] animate-pulse" />
      )}
    </span>
  )
}

function ProgressRing({ progress, size = 48, stroke = 4 }: { progress: number; size?: number; stroke?: number }) {
  const radius = (size - stroke) / 2
  const circumference = 2 * Math.PI * radius
  const offset = circumference - (progress / 100) * circumference
  
  return (
    <svg width={size} height={size} className="transform -rotate-90">
      <circle
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        stroke="var(--border)"
        strokeWidth={stroke}
      />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        stroke="url(#progress-gradient)"
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeDasharray={circumference}
        strokeDashoffset={offset}
        className="transition-all duration-500 ease-out"
        style={{ filter: 'drop-shadow(0 0 4px var(--primary-glow))' }}
      />
      <defs>
        <linearGradient id="progress-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor="var(--primary)" />
          <stop offset="100%" stopColor="var(--primary-dim)" />
        </linearGradient>
      </defs>
    </svg>
  )
}

function ProgressBar({ progress, className = '' }: { progress: number; className?: string }) {
  return (
    <div className={cn('progress-track', className)}>
      <div 
        className="progress-fill"
        style={{ width: `${Math.max(0, Math.min(100, progress))}%` }}
      />
    </div>
  )
}

function StatCard({ label, value, icon: Icon, trend, className = '' }: { 
  label: string
  value: string | number
  icon: React.ComponentType<{ className?: string }>
  trend?: string
  className?: string
}) {
  return (
    <div className={cn('glass p-4 rounded-2xl card-hover', className)}>
      <div className="flex items-start justify-between">
        <div>
          <p className="text-xs font-medium text-[var(--fg-muted)] uppercase tracking-wide">{label}</p>
          <p className="mt-1 text-2xl font-bold text-[var(--fg)]">{value}</p>
          {trend && (
            <p className="mt-1 text-xs text-[var(--success)] flex items-center gap-1">
              <ChevronRight className="w-3 h-3" />
              {trend}
            </p>
          )}
        </div>
        <div className="w-10 h-10 rounded-xl bg-[var(--primary)]/15 flex items-center justify-center text-[var(--primary)]">
          <Icon className="w-5 h-5" />
        </div>
      </div>
    </div>
  )
}

// Main Components
function Sidebar({ activeTab, onTabChange }: { activeTab: string; onTabChange: (tab: string) => void }) {
  const tabs = [
    { id: 'queue', label: 'Queue', icon: Download, badge: 3 },
    { id: 'clips', label: 'Clips', icon: Scissors, badge: 12 },
    { id: 'analytics', label: 'Analytics', icon: Trophy, badge: null },
    { id: 'templates', label: 'Templates', icon: Layers, badge: null },
    { id: 'assets', label: 'Assets', icon: Image, badge: 47 },
    { id: 'settings', label: 'Settings', icon: Settings, badge: null },
  ]

  return (
    <aside className="w-72 bg-[var(--bg-elevated)] border-r border-[var(--border)] flex flex-col">
      {/* Brand Header */}
      <div className="p-6 border-b border-[var(--border)]">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-[var(--primary)] to-[var(--accent)] flex items-center justify-center">
            <Zap className="w-6 h-6 text-[var(--bg)]" />
          </div>
          <div>
            <h1 className="text-xl font-bold text-[var(--fg)]">Clipper</h1>
            <p className="text-xs text-[var(--fg-subtle)]">Local Viral Clipper</p>
          </div>
        </div>
        <div className="mt-4 p-3 rounded-xl bg-[var(--bg)] border border-[var(--border)]">
          <div className="flex items-center gap-2 text-xs text-[var(--fg-muted)]">
            <span className="flex items-center gap-1">
              <span className="w-1.5 h-1.5 rounded-full bg-[var(--success)]" />
              Ready
            </span>
            <span className="text-[var(--border)]">·</span>
            <span>RTX 3050 4GB</span>
          </div>
        </div>
      </div>

      {/* Navigation */}
      <nav className="flex-1 p-4 space-y-1 overflow-y-auto">
        {tabs.map(tab => {
          const isActive = activeTab === tab.id
          const Icon = tab.icon
          return (
            <button
              key={tab.id}
              onClick={() => onTabChange(tab.id)}
              className={cn(
                'w-full flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-medium transition-all duration-200',
                isActive
                  ? 'bg-gradient-to-r from-[var(--primary)]/10 to-[var(--primary-dim)]/10 text-[var(--primary)] border border-[var(--primary)]/20'
                  : 'text-[var(--fg-muted)] hover:bg-[var(--bg)] hover:text-[var(--fg)] hover:border-[var(--border)]'
              )}
            >
              <Icon className={cn('w-5 h-5 flex-shrink-0', isActive && 'text-[var(--primary)]')} />
              <span className="flex-1 truncate">{tab.label}</span>
              {tab.badge && (
                <span className={cn(
                  'w-5 h-5 rounded-full text-xs font-bold flex items-center justify-center',
                  isActive ? 'bg-[var(--primary)] text-[var(--bg)]' : 'bg-[var(--bg)] text-[var(--fg-muted)] border border-[var(--border)]'
                )}>
                  {tab.badge}
                </span>
              )}
            </button>
          )
        })}
      </nav>

      {/* Footer */}
      <div className="p-4 border-t border-[var(--border)]">
        <div className="flex items-center justify-between text-xs text-[var(--fg-subtle)]">
          <span>v0.1.0 · Local-first</span>
          <span className="flex items-center gap-1">
            <span className="w-1.5 h-1.5 rounded-full bg-[var(--success)] animate-pulse" />
            <span>Online</span>
          </span>
        </div>
      </div>
    </aside>
  )
}

function Header({ onAddClick, stats }: { onAddClick: () => void; stats: { queued: number; processing: number; completed: number } }) {
  return (
    <header className="h-16 bg-[var(--bg-elevated)]/80 backdrop-blur-xl border-b border-[var(--border)] sticky top-0 z-10 flex items-center justify-between px-6">
      <div className="flex items-center gap-4">
        <h1 className="text-lg font-semibold text-[var(--fg)]">Processing Queue</h1>
        <div className="flex items-center gap-2">
          <StatusBadge status="pending" compact />
          <StatusBadge status="downloading" compact />
          <StatusBadge status="completed" compact />
        </div>
      </div>
      <div className="flex items-center gap-3">
        {/* Quick Stats */}
        <div className="hidden md:flex items-center gap-6">
          <div className="text-center">
            <p className="text-2xl font-bold text-[var(--primary)]">{stats.queued}</p>
            <p className="text-xs text-[var(--fg-subtle)]">Queued</p>
          </div>
          <div className="w-px h-8 bg-[var(--border)]" />
          <div className="text-center">
            <p className="text-2xl font-bold text-[var(--warning)]">{stats.processing}</p>
            <p className="text-xs text-[var(--fg-subtle)]">Processing</p>
          </div>
          <div className="w-px h-8 bg-[var(--border)]" />
          <div className="text-center">
            <p className="text-2xl font-bold text-[var(--success)]">{stats.completed}</p>
            <p className="text-xs text-[var(--fg-subtle)]">Done</p>
          </div>
        </div>
        
        <button onClick={onAddClick} className="btn-primary group">
          <Plus className="w-4 h-4 transition-transform group-hover:rotate-90" />
          <span>Add URL</span>
        </button>
      </div>
    </header>
  )
}

function EmptyState({ onAddClick }: { onAddClick: () => void }) {
  return (
    <div className="flex-1 flex items-center justify-center p-8">
      <div className="text-center max-w-md">
        <div className="w-24 h-24 mx-auto mb-6 rounded-2xl bg-gradient-to-br from-[var(--primary)]/20 to-[var(--accent)]/20 flex items-center justify-center">
          <Download className="w-12 h-12 text-[var(--primary)]" />
        </div>
        <h2 className="text-2xl font-bold text-[var(--fg)] mb-2">Ready to Create Viral Shorts</h2>
        <p className="text-[var(--fg-muted)] mb-8 leading-relaxed">
          Drop a YouTube URL and let AI find the best moments. 
          Get 3 vertical clips with captions, zoom effects, and viral scoring — all 100% local.
        </p>
        <div className="flex items-center justify-center gap-4 flex-wrap">
          <button onClick={onAddClick} className="btn-primary group w-full sm:w-auto">
            <Plus className="w-4 h-4" />
            Add Your First URL
          </button>
          <button className="btn-secondary w-full sm:w-auto">
            <FolderOpen className="w-4 h-4" />
            Browse Local File
          </button>
        </div>
        
        {/* Features preview */}
        <div className="mt-12 grid grid-cols-3 gap-4 text-center">
          {[
            { icon: Brain, label: 'AI Scoring', desc: 'Hybrid heuristics + LLM' },
            { icon: Wand2, label: 'Creative Effects', desc: 'Zoom, captions, jump cuts' },
            { icon: Film, label: 'NVENC Render', desc: 'Hardware accelerated' },
          ].map((item, i) => (
            <div key={i} className="p-4 glass rounded-xl">
              <item.icon className="w-6 h-6 mx-auto mb-2 text-[var(--primary)]" />
              <p className="text-sm font-medium text-[var(--fg)]">{item.label}</p>
              <p className="text-xs text-[var(--fg-subtle)]">{item.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

function ProjectCard({ project, onRemove, onOpen }: { 
  project: Project
  onRemove: () => void
  onOpen: () => void
}) {
  const config = STAGE_CONFIG[project.status]
  const isActive = ['downloading', 'transcribing', 'analyzing', 'rendering'].includes(project.status)
  const isComplete = project.status === 'completed'
  
  return (
    <div 
      className={cn(
        'glass rounded-2xl border border-[var(--border)] p-4 transition-all duration-300',
        'hover:border-[var(--border-hover)] hover:shadow-xl hover:shadow-[var(--primary)]/5',
        isActive && 'ring-1 ring-[var(--primary)]/30 animate-pulse-glow',
        isComplete && 'ring-1 ring-[var(--success)]/30'
      )}
      onClick={onOpen}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          {/* Header */}
          <div className="flex items-center justify-between gap-2 mb-3">
            <div className="flex items-center gap-3 flex-1 min-w-0">
              {project.thumbnail && (
                <div className="w-12 h-12 rounded-xl overflow-hidden flex-shrink-0 bg-[var(--bg)]">
                  <img src={project.thumbnail} alt="" className="w-full h-full object-cover" />
                </div>
              )}
              <div className="min-w-0">
                <h3 className="font-medium text-[var(--fg)] truncate">{project.title || 'Untitled'}</h3>
                <p className="text-xs text-[var(--fg-subtle)] truncate">{project.url}</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <StatusBadge status={project.status} />
              <button 
                onClick={(e) => { e.stopPropagation(); onRemove(); }}
                className="btn-icon text-[var(--fg-subtle)] hover:text-[var(--danger)] hover:bg-[var(--danger)]/10 opacity-0 group-hover:opacity-100 transition-all"
                aria-label="Remove project"
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
          </div>

          {/* Progress */}
          <div className="mb-3">
            <div className="flex items-center justify-between text-xs text-[var(--fg-subtle)] mb-1.5">
              <span>{config.label}</span>
              <span className="font-semibold text-[var(--fg)]">{Math.round(project.progress)}%</span>
            </div>
            <ProgressBar progress={project.progress} />
            {isActive && (
              <p className="mt-1.5 text-xs text-[var(--fg-muted)] flex items-center gap-1">
                <Loader2 className="w-3 h-3 animate-spin" />
                {project.stage}
              </p>
            )}
          </div>

          {/* Clips preview */}
          {project.clips && project.clips.length > 0 && (
            <div className="border-t border-[var(--border)] pt-3 mt-3 animate-slide-up">
              <p className="text-xs font-medium text-[var(--fg-muted)] mb-2">Generated Clips ({project.clips.length})</p>
              <div className="flex gap-2 overflow-x-auto scrollbar-thin pb-1">
                {project.clips.map((clip) => (
                  <div 
                    key={clip.id} 
                    className="flex-shrink-0 w-44 glass p-3 rounded-xl border border-[var(--border)] card-hover"
                  >
                    <p className="text-xs font-medium text-[var(--fg)] truncate mb-1">{clip.title}</p>
                    <p className="text-xs text-[var(--fg-subtle)] mb-2">{clip.start}s – {clip.end}s</p>
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-1">
                        <Trophy className="w-3 h-3 text-[var(--warning)]" />
                        <span className="text-xs font-semibold text-[var(--fg)]">{clip.viralScore}/10</span>
                      </div>
                      {clip.renderPath && (
                        <span className="badge-success text-xs">Ready</span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Error */}
          {project.error && (
            <div className="mt-3 p-3 rounded-xl bg-[var(--danger)]/10 border border-[var(--danger)]/20 animate-slide-down">
              <div className="flex items-center gap-2 text-sm text-[var(--danger)]">
                <AlertTriangle className="w-4 h-4 flex-shrink-0" />
                <span className="font-medium">Error</span>
              </div>
              <p className="mt-1 text-xs text-[var(--fg-muted)]">{project.error}</p>
              <button className="mt-2 btn-secondary text-xs">
                <RefreshCw className="w-3 h-3" />
                Retry
              </button>
            </div>
          )}
        </div>

        {/* Side progress ring for active projects */}
        {isActive && (
          <div className="flex-shrink-0 ml-4">
            <ProgressRing progress={project.progress} size={56} stroke={5} />
          </div>
        )}
      </div>
    </div>
  )
}

function AddProjectModal({ isOpen, onClose, onSubmit }: { 
  isOpen: boolean
  onClose: () => void
  onSubmit: (url: string, options: { template: string; targetClips: number; context: string }) => void
}) {
  const [url, setUrl] = useState('')
  const [loading, setLoading] = useState(false)
  const [template, setTemplate] = useState('tiktok')
  const [targetClips, setTargetClips] = useState(3)
  const [context, setContext] = useState('')

  if (!isOpen) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-[var(--bg)]/80 backdrop-blur-sm animate-fade-in">
      <div className="glass-strong w-full max-w-lg rounded-2xl border border-[var(--border)] overflow-hidden animate-scale-in">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[var(--border)]">
          <h2 className="text-lg font-semibold text-[var(--fg)]">Add YouTube URL</h2>
          <button onClick={onClose} className="btn-icon text-[var(--fg-subtle)] hover:text-[var(--fg)] hover:bg-[var(--bg-elevated)]">
            <X className="w-5 h-5" />
          </button>
        </div>

        <form onSubmit={(e) => { e.preventDefault(); if (url.trim() && !loading) { setLoading(true); onSubmit(url.trim(), { template, targetClips, context }); setUrl(''); onClose(); setLoading(false); } }} className="p-4 space-y-4">
          {/* URL Input */}
          <div>
            <label className="label">YouTube URL</label>
            <div className="relative">
              <input
                type="url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://youtube.com/watch?v=... or https://youtu.be/..."
                className="input pl-10"
                autoFocus
                required
                disabled={loading}
              />
              <Globe className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-[var(--fg-subtle)]" />
            </div>
            <p className="mt-1 text-xs text-[var(--fg-subtle)]">Supports youtube.com, youtu.be, and youtube.com/shorts/</p>
          </div>

          {/* Template Select */}
          <div>
            <label className="label">Caption Template</label>
            <div className="grid grid-cols-3 gap-2">
              {TEMPLATES.map(t => (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => setTemplate(t.id)}
                  className={cn(
                    'relative p-3 rounded-xl border-2 transition-all duration-200 text-left',
                    template === t.id
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5'
                      : 'border-[var(--border)] hover:border-[var(--primary)]/50'
                  )}
                >
                  <input type="radio" name="template" value={t.id} checked={template === t.id} className="absolute inset-0 opacity-0 cursor-pointer" />
                  <div className="flex items-center justify-center gap-2">
                    <span className="text-2xl">{t.preview}</span>
                    <span className="text-sm font-medium text-[var(--fg)]">{t.name}</span>
                  </div>
                  <p className="mt-1 text-xs text-[var(--fg-subtle)] text-center">{t.description}</p>
                  {template === t.id && (
                    <div className="absolute top-2 right-2 w-5 h-5 rounded-full bg-[var(--primary)] flex items-center justify-center">
                      <Check className="w-3 h-3 text-[var(--bg)]" />
                    </div>
                  )}
                </button>
              ))}
            </div>
          </div>

          {/* Options */}
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="label">Clips to Generate</label>
              <select
                value={targetClips}
                onChange={(e) => setTargetClips(Number(e.target.value))}
                className="input appearance-none bg-[var(--input-bg)]"
                disabled={loading}
              >
                {[1, 2, 3, 4, 5].map(n => (
                  <option key={n} value={n}>{n} Clip{n > 1 ? 's' : ''}</option>
                ))}
              </select>
            </div>
            <div>
              <label className="label">Channel Context (optional)</label>
              <input
                type="text"
                value={context}
                onChange={(e) => setContext(e.target.value)}
                placeholder="e.g. Tech tutorials, gaming, finance"
                className="input"
                disabled={loading}
              />
            </div>
          </div>

          {/* Actions */}
          <div className="flex gap-3 justify-end pt-2">
            <button type="button" onClick={onClose} className="btn-secondary" disabled={loading}>
              Cancel
            </button>
            <button type="submit" disabled={!url.trim() || loading} className="btn-primary">
              {loading ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Adding...
                </>
              ) : (
                <>
                  <Plus className="w-4 h-4" />
                  Add to Queue
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}

function QueueView({ projects, onRemove, onOpen, onAddClick }: { 
  projects: Project[]
  onRemove: (id: string) => void
  onOpen: (project: Project) => void
  onAddClick: () => void
}) {
  const stats = {
    queued: projects.filter(p => p.status === 'pending').length,
    processing: projects.filter(p => ['downloading', 'transcribing', 'analyzing', 'rendering'].includes(p.status)).length,
    completed: projects.filter(p => p.status === 'completed').length,
  }

  return (
    <div className="flex-1 flex flex-col min-w-0">
      <Header stats={stats} onAddClick={onAddClick} />
      <main className="flex-1 overflow-y-auto p-6">
        <div className="max-w-5xl mx-auto w-full">
          {projects.length === 0 ? (
            <EmptyState onAddClick={onAddClick} />
          ) : (
            <div className="space-y-4">
              {projects.map(project => (
                <ProjectCard
                  key={project.id}
                  project={project}
                  onRemove={() => onRemove(project.id)}
                  onOpen={() => onOpen(project)}
                />
              ))}
            </div>
          )}
        </div>
      </main>
    </div>
  )
}

function ClipDetailModal({ clip, onClose }: { clip: Clip; onClose: () => void }) {
  if (!clip) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-[var(--bg)]/90 backdrop-blur-sm animate-fade-in">
      <div className="glass-strong w-full max-w-4xl max-h-[90vh] rounded-2xl border border-[var(--border)] overflow-hidden animate-scale-in flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[var(--border)]">
          <div className="flex items-center gap-3">
            <div className="w-12 h-12 rounded-xl bg-gradient-to-br from-[var(--primary)] to-[var(--accent)] flex items-center justify-center">
              <Film className="w-6 h-6 text-[var(--bg)]" />
            </div>
            <div>
              <h2 className="font-semibold text-[var(--fg)]">{clip.title}</h2>
              <p className="text-xs text-[var(--fg-subtle)]">{clip.start}s – {clip.end}s · {clip.duration}s</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <StatusBadge status="completed" compact />
            <button onClick={onClose} className="btn-icon text-[var(--fg-subtle)] hover:text-[var(--fg)] hover:bg-[var(--bg-elevated)]">
              <X className="w-5 h-5" />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-4 space-y-6">
          {/* Video Preview Placeholder */}
          <div className="aspect-video rounded-xl bg-[var(--bg)] border border-[var(--border)] flex items-center justify-center">
            <div className="text-center text-[var(--fg-subtle)]">
              <Play className="w-12 h-12 mx-auto mb-2 text-[var(--fg-muted)]" />
              <p>Video preview would appear here</p>
              <p className="text-xs mt-1">{clip.renderPath}</p>
            </div>
          </div>

          {/* Scores */}
          <div className="grid grid-cols-2 gap-3">
            <StatCard
              label="Viral Score"
              value={`${clip.viralScore}/10`}
              icon={Trophy}
              trend={clip.viralScore >= 8 ? 'High potential' : clip.viralScore >= 6 ? 'Good' : 'Needs work'}
            />
            <StatCard
              label="Heuristic Score"
              value={`${Math.round(clip.heuristicScore * 100)}%`}
              icon={Brain}
            />
          </div>

          {/* Breakdown */}
          <div>
            <h3 className="text-sm font-medium text-[var(--fg)] mb-3 flex items-center gap-2">
              <Brain className="w-4 h-4 text-[var(--primary)]" />
              Score Breakdown
            </h3>
            <div className="space-y-2">
              {Object.entries(clip.breakdown).map(([key, value]) => (
                <div key={key} className="flex items-center gap-3">
                  <span className="w-24 text-xs text-[var(--fg-muted)] capitalize">{key.replace('_', ' ')}</span>
                  <div className="flex-1 h-2 bg-[var(--border)] rounded-full overflow-hidden">
                    <div 
                      className="h-full bg-gradient-to-r from-[var(--primary)] to-[var(--primary-dim)] rounded-full transition-all duration-500"
                      style={{ width: `${value * 100}%` }}
                    />
                  </div>
                  <span className="w-10 text-xs font-mono text-[var(--fg)] text-right">{Math.round(value * 100)}%</span>
                </div>
              ))}
            </div>
          </div>

          {/* AI Reason */}
          <div className="glass p-4 rounded-xl border border-[var(--border)]">
            <h3 className="text-sm font-medium text-[var(--fg)] mb-2 flex items-center gap-2">
              <Sparkles className="w-4 h-4 text-[var(--accent)]" />
              AI Analysis
            </h3>
            <p className="text-sm text-[var(--fg-muted)] leading-relaxed">{clip.reason}</p>
          </div>

          {/* Hook & Transcript */}
          <div className="grid grid-cols-2 gap-4">
            <div className="glass p-4 rounded-xl border border-[var(--border)]">
              <h3 className="text-sm font-medium text-[var(--fg)] mb-2 flex items-center gap-2">
                <ZapIcon className="w-4 h-4 text-[var(--warning)]" />
                Suggested Hook
              </h3>
              <p className="text-sm text-[var(--fg)] italic bg-[var(--bg)] p-3 rounded-lg border border-[var(--border)]">"{clip.hook}"</p>
            </div>
            <div className="glass p-4 rounded-xl border border-[var(--border)]">
              <h3 className="text-sm font-medium text-[var(--fg)] mb-2 flex items-center gap-2">
                <Hash className="w-4 h-4 text-[var(--primary)]" />
                Transcript Excerpt
              </h3>
              <p className="text-sm text-[var(--fg-muted)] leading-relaxed line-clamp-4">{clip.text}</p>
            </div>
          </div>

          {/* Actions */}
          <div className="flex items-center justify-end gap-3 pt-4 border-t border-[var(--border)]">
            <button className="btn-secondary">
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
            <button className="btn-secondary">
              <Copy className="w-4 h-4" />
              Copy Path
            </button>
            <button className="btn-secondary">
              <Share2 className="w-4 h-4" />
              Share
            </button>
            {clip.renderPath && (
              <a href={clip.renderPath} target="_blank" rel="noopener noreferrer" className="btn-primary">
                <Maximize2 className="w-4 h-4" />
                Open Video
              </a>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

// Main App
export default function App() {
  const [projects, setProjects] = useState<Project[]>([])
  const [activeTab, setActiveTab] = useState('queue')
  const [showAddModal, setShowAddModal] = useState(false)
  const [selectedProject, setSelectedProject] = useState<Project | null>(null)
  const [selectedClip, setSelectedClip] = useState<Clip | null>(null)

  const handleAddProject = useCallback((url: string, options: { template: string; targetClips: number; context: string }) => {
    const newProject: Project = {
      id: crypto.randomUUID(),
      url,
      title: 'Fetching video info...',
      status: 'pending',
      progress: 0,
      stage: 'Initializing...',
      createdAt: new Date(),
    }
    setProjects(prev => [newProject, ...prev])
    simulatePipeline(newProject.id, options)
  }, [])

  const simulatePipeline = async (projectId: string, options: { template: string; targetClips: number; context: string }) => {
    const stages: Array<{ status: Project['status']; stage: string; progress: number; delay: number }> = [
      { status: 'downloading', stage: 'Downloading video...', progress: 15, delay: 2000 },
      { status: 'transcribing', stage: 'Transcribing with Whisper...', progress: 35, delay: 3000 },
      { status: 'analyzing', stage: 'AI analyzing best moments...', progress: 60, delay: 2500 },
      { status: 'rendering', stage: 'Rendering clips with effects...', progress: 80, delay: 3000 },
    ]

    for (const stage of stages) {
      await new Promise(r => setTimeout(r, stage.delay + Math.random() * 1000))
      setProjects(prev => prev.map(p => 
        p.id === projectId ? { ...p, status: stage.status, stage: stage.stage, progress: stage.progress } : p
      ))
    }

    // Complete with mock clips
    await new Promise(r => setTimeout(r, 1000))
    const mockClips: Clip[] = [
      { 
        id: `${projectId}_clip_1`, start: 15, end: 45, duration: 30,
        title: 'The Hook That Gets 1M Views', hook: 'Stop scrolling! This changes everything...',
        viralScore: 9.2, heuristicScore: 0.87,
        reason: 'Exceptional hook density in first 3s, perfect pace at 3.2 wps, strong sentiment arc with 2 shifts, self-contained narrative.',
        breakdown: { hook: 0.95, pace: 0.88, sentiment: 0.82, containment: 0.9, audio_energy: 0.75, duration: 1.0 },
        text: 'Stop scrolling! This changes everything about how you think about viral content. The secret isn\'t luck — it\'s a formula.',
        status: 'completed' as const,
        renderPath: '/output/clip_1.mp4'
      },
      { 
        id: `${projectId}_clip_2`, start: 120, end: 155, duration: 35,
        title: 'The Secret Sauce Revealed', hook: 'Here\'s the trick nobody tells you...',
        viralScore: 8.7, heuristicScore: 0.82,
        reason: 'Strong curiosity gap hook, good pace, positive sentiment shift, clear entity references.',
        breakdown: { hook: 0.88, pace: 0.85, sentiment: 0.78, containment: 0.85, audio_energy: 0.8, duration: 1.0 },
        text: 'Here\'s the trick nobody tells you about retention editing. Most creators cut too early. You need to let the tension breathe.',
        status: 'completed' as const,
        renderPath: '/output/clip_2.mp4'
      },
      { 
        id: `${projectId}_clip_3`, start: 300, end: 340, duration: 40,
        title: 'Results After 30 Days', hook: 'You won\'t believe what happened...',
        viralScore: 8.5, heuristicScore: 0.79,
        reason: 'Strong negative framing hook, steady pace, resolved sentiment arc, concrete results mentioned.',
        breakdown: { hook: 0.85, pace: 0.8, sentiment: 0.75, containment: 0.88, audio_energy: 0.7, duration: 1.0 },
        text: 'You won\'t believe what happened after 30 days of using this exact method. My views went from 500 to 500,000.',
        status: 'completed' as const,
        renderPath: '/output/clip_3.mp4'
      },
    ].slice(0, options.targetClips)

    setProjects(prev => prev.map(p => 
      p.id === projectId ? { 
        ...p, 
        status: 'completed', 
        stage: 'Done!', 
        progress: 100,
        title: 'How I Built a Viral Clipper in 6 Weeks',
        clips: mockClips 
      } : p
    ))
  }

  const handleRemove = useCallback((id: string) => {
    setProjects(prev => prev.filter(p => p.id !== id))
  }, [])

  const handleOpenProject = useCallback((project: Project) => {
    setSelectedProject(project)
  }, [])

  const handleOpenClip = useCallback((clip: Clip) => {
    setSelectedClip(clip)
  }, [])

  return (
    <div className="flex h-screen bg-[var(--bg)] text-[var(--fg)]">
      <Sidebar activeTab={activeTab} onTabChange={setActiveTab} />
      
      {activeTab === 'queue' && (
        <QueueView
          projects={projects}
          onRemove={handleRemove}
          onOpen={handleOpenProject}
          onAddClick={() => setShowAddModal(true)}
        />
      )}

      {activeTab !== 'queue' && (
        <div className="flex-1 flex items-center justify-center p-8">
          <div className="text-center">
            <div className="w-16 h-16 mx-auto mb-4 rounded-xl bg-[var(--bg-elevated)] flex items-center justify-center text-[var(--fg-muted)]">
              <Layers className="w-8 h-8" />
            </div>
            <h2 className="text-xl font-semibold text-[var(--fg)] mb-2">Coming Soon</h2>
            <p className="text-[var(--fg-muted)]">This tab is under development</p>
          </div>
        </div>
      )}

      <AddProjectModal
        isOpen={showAddModal}
        onClose={() => setShowAddModal(false)}
        onSubmit={handleAddProject}
      />

      {selectedProject && selectedClip && (
        <ClipDetailModal
          clip={selectedClip}
          project={selectedProject}
          onClose={() => { setSelectedClip(null); setSelectedProject(null); }}
        />
      )}
    </div>
  )
}