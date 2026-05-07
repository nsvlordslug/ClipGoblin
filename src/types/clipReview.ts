export type ClipReviewRating = 'good' | 'meh' | 'boring';

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
