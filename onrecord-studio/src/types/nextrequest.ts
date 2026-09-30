// INVARIANT(source): This module contains publisher-specific NextRequest wire types; normalized
// cross-source domain types remain in investigation.ts and source.ts.
export interface PaginationMeta {
  current_page: number;
  next_page: number | null;
  prev_page: number | null;
  total_pages: number;
  total_count: number;
}

export interface AgencyMetadata {
  id: number;
  name: string;
  department_name: string | null;
}

export interface CategoryMetadata {
  id: number;
  name: string;
}

export interface DocumentMetadata {
  id: number;
  file_name: string | null;
  file_size: number | null; // Stored cleanly in bytes
  description: string | null;
  content_type: string | null;
  download_url: string | null;
  created_at: string; // ISO-8601 string date
  updated_at: string;
  visibility: 'public' | 'staff_only' | string;
  
  // Pipeline tracking fields injected by Rust core
  local_path?: string | null;
  extracted_text_sha256?: string | null;
  was_ocr_processed?: boolean;
}

export interface PublicRequest {
  id: number;
  rpid: string | null; // Alphanumeric agency tracking code
  title: string | null;
  description: string | null;
  status: 'open' | 'closed' | 'referred' | string;
  current_status_id: number | null;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
  due_date: string | null;
  
  // Relations
  documents: DocumentMetadata[];
  agency: AgencyMetadata | null;
  category: CategoryMetadata | null;
  
  // Custom timeline exceptions derived by Rust processing logic
  timeline_anomalies?: string[];
}
