export type ClipReviewRating = 'good' | 'meh' | 'boring';

export type ClipReviewIssue =
  | 'starts_too_late'
  | 'cuts_off_early'
  | 'too_long'
  | 'wrong_moment'
  | 'duplicate';

export type PersonalizationState =
  | 'empty'
  | 'needs_more'
  | 'needs_variety'
  | 'learning'
  | 'active';

export interface PersonalizationStatus {
  state: PersonalizationState;
  total_ratings: number;
  usable_ratings: number;
  rating_classes: number;
  confidence: number;
  is_personalizing: boolean;
  target_ratings: number;
  behavior_events?: number;
  usable_behavior_events?: number;
  total_evidence?: number;
  boundary_feedback_samples?: number;
  boundary_learning_active?: boolean;
  boundary_confidence?: number;
}

export interface PersonalizationStatusCopy {
  label: string;
  detail: string;
  tone: 'neutral' | 'attention' | 'learning' | 'active';
}

export const REVIEW_RATING_LABELS: Record<ClipReviewRating, string> = {
  good: '✓ Good',
  meh: '— Meh',
  boring: '✗ Boring',
};

/** Tailwind classes for the rating badge color, used for both the inline button highlight and the corner badge. */
export const REVIEW_RATING_COLORS: Record<ClipReviewRating, string> = {
  good: 'bg-emerald-500/20 text-emerald-300 border-emerald-500/30',
  meh: 'bg-slate-500/20 text-slate-300 border-slate-500/30',
  boring: 'bg-rose-500/20 text-rose-300 border-rose-500/30',
};

export const REVIEW_ISSUE_OPTIONS: ReadonlyArray<{
  id: ClipReviewIssue;
  label: string;
}> = [
  { id: 'starts_too_late', label: 'Starts too late' },
  { id: 'cuts_off_early', label: 'Cuts off early' },
  { id: 'too_long', label: 'Too long' },
  { id: 'wrong_moment', label: 'Wrong moment' },
  { id: 'duplicate', label: 'Duplicate' },
];

const REVIEW_ISSUE_IDS = new Set<ClipReviewIssue>(
  REVIEW_ISSUE_OPTIONS.map((option) => option.id),
);

export function parseClipReviewIssues(value?: string | null): ClipReviewIssue[] {
  if (!value) return [];
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed)) return [];
    const unique = new Set<ClipReviewIssue>();
    for (const issue of parsed) {
      if (typeof issue === 'string' && REVIEW_ISSUE_IDS.has(issue as ClipReviewIssue)) {
        unique.add(issue as ClipReviewIssue);
      }
    }
    return [...unique];
  } catch {
    return [];
  }
}

export function toggleExpandedReviewClip(
  currentClipId: string | null,
  requestedClipId: string,
): string | null {
  return currentClipId === requestedClipId ? null : requestedClipId;
}

export function getPersonalizationStatusCopy(
  status: PersonalizationStatus,
): PersonalizationStatusCopy {
  const usable = status.usable_ratings;
  const behavior = status.usable_behavior_events ?? 0;
  const totalEvidence = status.total_evidence ?? usable;
  const target = Math.max(status.target_ratings, 1);
  const confidence = Math.round(Math.max(0, Math.min(status.confidence, 1)) * 100);
  const boundarySamples = status.boundary_feedback_samples ?? 0;
  const boundarySuffix = status.boundary_learning_active
    ? ` Boundary timing is also learning from ${boundarySamples} edited clips.`
    : '';

  if (status.boundary_learning_active && !status.is_personalizing) {
    return {
      label: 'Boundary learning is active',
      detail: `${boundarySamples} clips are teaching ClipGoblin where your clips should start and end. Add varied Good, Meh, or Boring ratings to personalize ranking too.`,
      tone: 'learning',
    };
  }

  switch (status.state) {
    case 'needs_more':
      return {
        label: 'Learning not active yet',
        detail: behavior > 0
          ? `${totalEvidence}/4 usable signals (${usable} ratings, ${behavior} actions). Add a few more Good, Meh, or Boring choices.`
          : `${usable}/4 usable ratings. Add a few more Good, Meh, or Boring choices.`,
        tone: 'attention',
      };
    case 'needs_variety':
      return {
        label: 'More rating variety needed',
        detail: `${usable} usable ratings. Use at least two rating choices so ClipGoblin can learn a contrast.`,
        tone: 'attention',
      };
    case 'learning':
      return {
        label: 'Personalization is learning',
        detail: behavior > 0
          ? `${usable}/${target} ratings plus ${behavior} useful actions, ${confidence}% confidence. Future analyses are already being gently reordered.${boundarySuffix}`
          : `${usable}/${target} usable ratings, ${confidence}% confidence. Future analyses are already being gently reordered.${boundarySuffix}`,
        tone: 'learning',
      };
    case 'active':
      return {
        label: 'Personalization is active',
        detail: behavior > 0
          ? `${usable} ratings plus ${behavior} useful actions, ${confidence}% confidence. New feedback keeps refining your local profile.${boundarySuffix}`
          : `${usable} usable ratings, ${confidence}% confidence. New feedback keeps refining your local profile.${boundarySuffix}`,
        tone: 'active',
      };
    default:
      return {
        label: 'Ready to learn your taste',
        detail: 'Rate clips Good, Meh, or Boring. Feedback stays on this PC and applies to future analyses.',
        tone: 'neutral',
      };
  }
}
