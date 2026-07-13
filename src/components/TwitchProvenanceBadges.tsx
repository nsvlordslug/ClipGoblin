import { AudioLines, Radio, Sparkles, Star, Users } from 'lucide-react'
import Tooltip from './Tooltip'
import { deriveTwitchProvenance } from '../lib/twitchProvenance'
import type { TwitchProvenanceKind } from '../lib/twitchProvenance'

interface Props {
  tags: unknown
  signalSources?: unknown
  compact?: boolean
  className?: string
}

const BADGE_STYLES: Record<TwitchProvenanceKind, string> = {
  twitch: 'border-cyan-500/30 bg-cyan-500/12 text-cyan-300',
  streamer: 'border-cyan-500/30 bg-cyan-500/12 text-cyan-300',
  viewer: 'border-slate-500/40 bg-slate-500/15 text-slate-300',
  featured: 'border-amber-500/35 bg-amber-500/12 text-amber-300',
  consensus: 'border-emerald-500/35 bg-emerald-500/12 text-emerald-300',
  local: 'border-sky-500/35 bg-sky-500/12 text-sky-300',
  ai: 'border-violet-500/35 bg-violet-500/12 text-violet-300',
}

function BadgeIcon({ kind }: { kind: TwitchProvenanceKind }) {
  const className = 'w-3 h-3 shrink-0'
  if (kind === 'featured') return <Star className={className} />
  if (kind === 'consensus') return <Users className={className} />
  if (kind === 'local') return <AudioLines className={className} />
  if (kind === 'ai') return <Sparkles className={className} />
  return <Radio className={className} />
}

export default function TwitchProvenanceBadges({
  tags,
  signalSources,
  compact = false,
  className = '',
}: Props) {
  const badges = deriveTwitchProvenance(tags, signalSources)
  if (badges.length === 0) return null

  return (
    <div className={`flex flex-wrap items-center gap-1.5 ${className}`.trim()}>
      {badges.map(badge => (
        <Tooltip key={badge.kind} text={badge.tooltip} position="bottom" delay={250}>
          <span
            className={`inline-flex h-5 items-center gap-1 whitespace-nowrap rounded-full border px-1.5 text-[10px] font-medium ${BADGE_STYLES[badge.kind]}`}
          >
            <BadgeIcon kind={badge.kind} />
            {compact ? badge.compactLabel : badge.label}
          </span>
        </Tooltip>
      ))}
    </div>
  )
}
