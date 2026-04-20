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
  facecam_layout: string;
  render_status: 'pending' | 'rendering' | 'completed' | 'failed';
  output_path: string | null;
  thumbnail_path: string | null;
  game: string | null;
  publish_description: string | null;
  publish_hashtags: string | null;
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
  status: 'pending' | 'uploading' | 'completed' | 'failed'
  retry_count: number
  error_message: string | null
  video_url: string | null
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
