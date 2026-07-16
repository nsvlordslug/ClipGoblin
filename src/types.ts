export interface TwitchChannel {
  id: string;
  twitch_user_id: string;
  twitch_login: string;
  display_name: string;
  profile_image_url: string;
}

export interface Vod {
  id: string;
  channel_id: string;
  twitch_video_id: string;
  title: string;
  duration_seconds: number;
  stream_date: string;
  thumbnail_url: string;
  download_status: 'pending' | 'downloading' | 'downloaded' | 'failed';
  download_progress: number;
  analysis_status: 'pending' | 'analyzing' | 'completed' | 'failed';
  analysis_progress: number;
  local_path: string | null;
  game_name: string | null;
  cam_region_norm: string | null;
}

export interface Highlight {
  id: string;
  vod_id: string;
  start_seconds: number;
  end_seconds: number;
  virality_score: number;
  audio_score: number;
  visual_score: number;
  chat_score: number;
  transcript_snippet: string;
  description: string;
  tags: string[];
  /** Calibrated user-facing confidence (0.0–0.98). Null for pre-migration rows. */
  confidence_score?: number | null;
  /** Factual explanation of why this highlight was selected. Null for pre-migration rows. */
  explanation?: string | null;
  /** One-sentence event summary (what happened). Null for pre-migration rows. */
  event_summary?: string | null;
  /** JSON-serialized 6-dimension scoring breakdown. Null for pre-Phase-C rows. */
  scoring_dimensions?: string | null;
  /** JSON-serialized array of signal-source identifiers. Null for pre-Phase-C rows. */
  signal_sources?: string | null;
  /** User-supplied moment rating. Null if unrated. */
  review_rating?: 'good' | 'meh' | 'boring' | null;
  /** Free-form user review note. Null if no note is set. */
  review_note?: string | null;
  /** JSON-serialized multi-select edit-quality issues. */
  review_issues?: string | null;
}

export interface Clip {
  id: string;
  highlight_id: string;
  vod_id: string;
  title: string;
  start_seconds: number;
  end_seconds: number;
  aspect_ratio: string;
  crop_x: number | null;
  crop_y: number | null;
  crop_width: number | null;
  crop_height: number | null;
  captions_enabled: number;
  captions_text: string | null;
  captions_position: string;
  caption_style: string;
  caption_font_scale: number;
  caption_y_offset?: number;
  /** Absolute source-media timestamp represented by SRT time 0:00. */
  captions_source_start?: number | null;
  facecam_layout: string;
  facecam_settings?: string | null;
  context_background_path?: string | null;
  context_background_mode?: 'blur' | 'branding';
  context_blur_strength?: number;
  context_video_y?: number;
  render_status: 'pending' | 'rendering' | 'completed' | 'failed';
  output_path: string | null;
  /**
   * When set, this clip's video is a STANDALONE, already-trimmed MP4 (a
   * downloaded Twitch community clip) rather than a sub-range of the VOD.
   * Players must use this file directly and treat it as the whole clip
   * (0 → its own duration) — NOT seek/trim to start_seconds/end_seconds.
   * Null/absent = normal VOD-seek behavior (unchanged).
   */
  community_clip_mp4_path?: string | null;
  /** Editable standalone media imported from a local source. */
  source_kind?: 'twitch_vod' | 'twitch_community' | 'medal' | 'obs' | 'meld' | 'manual' | string;
  source_media_path?: string | null;
  source_fingerprint?: string | null;
  source_recorded_at?: string | null;
  thumbnail_path: string | null;
  game: string | null;
  publish_description: string | null;
  publish_hashtags: string | null;
  cam_region_norm_override: string | null;
  cam_fit_mode: 'fit' | 'fill' | 'stretch' | null;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  storage_used: string;
}

export interface ScheduledUpload {
  id: string
  clip_id: string
  platform: string
  scheduled_time: string
  status: 'pending' | 'uploading' | 'processing' | 'completed' | 'failed' | 'cancelled'
  retry_count: number
  error_message: string | null
  video_url: string | null
  job_id: string | null
  platform_video_id: string | null
  upload_meta_json: string | null
  created_at: string
  /** Views reported by the platform's API. null = never fetched. */
  view_count?: number | null
  /** Likes reported by the platform. null = never fetched or platform doesn't expose this. */
  like_count?: number | null
  /** Click-through rate as a percentage (0-100). YouTube-only. */
  ctr_percent?: number | null
  /** ISO8601 timestamp of the last successful stats refresh. */
  stats_updated_at?: string | null
}
