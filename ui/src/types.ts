// What the server sends about assets (Ferrum docs/api.md): money and decimals as strings, dates as
// `YYYY-MM-DD`, links as ids with their words in `_titles`. Only the fields these screens read.

export interface ScheduleRow {
  id: string;
  schedule_date: string;
  depreciation_amount: string;
  accumulated_depreciation?: string | null;
  booked?: boolean;
  journal_entry?: string | null;
  disposal?: string | null;
  posting_error?: string | null;
}

export interface Asset {
  id: string;
  version?: number;
  docstatus?: number;
  number?: string | null;
  asset_name?: string | null;
  item?: string | null;
  asset_category?: string | null;
  company?: string | null;
  location?: string | null;
  custodian?: string | null;
  expected_return_date?: string | null;
  status?: string | null;
  asset_quantity?: number | null;
  serial_number?: string | null;
  manufacturer?: string | null;
  model_number?: string | null;
  warranty_expiry_date?: string | null;
  end_of_life_date?: string | null;
  purchase_date?: string | null;
  available_for_use_date?: string | null;
  purchase_invoice?: string | null;
  is_existing_asset?: boolean;
  gross_purchase_amount?: string | null;
  opening_accumulated_depreciation?: string | null;
  salvage_value?: string | null;
  total_asset_cost?: string | null;
  additional_asset_cost?: string | null;
  value_adjustments?: string | null;
  accumulated_depreciation?: string | null;
  value_after_depreciation?: string | null;
  calculate_depreciation?: boolean;
  depreciation_method?: string | null;
  next_depreciation_date?: string | null;
  last_verified_date?: string | null;
  next_verification_date?: string | null;
  verification_result?: string | null;
  disposal?: string | null;
  disposal_date?: string | null;
  description?: string | null;
  schedule?: ScheduleRow[];
  _titles?: Record<string, string>;
}

/** A row of a report: its columns are named by the report, these are the ones read here. */
export interface Report<Row> {
  rows: Row[];
  totals?: Record<string, string | number> | null;
  meta?: { company?: string; currency?: string; as_of?: string; truncated?: boolean; [k: string]: unknown } | null;
}

/** The fixed asset register, grouped by category or by location. */
export interface RegisterRow {
  asset_category?: string | null;
  location?: string | null;
  assets?: number;
  cost: string;
  accumulated_depreciation: string;
  net_book_value: string;
}

/** The depreciation schedule report: one row per asset and period. */
export interface PlanRow {
  asset?: string | null;
  asset_id: string;
  asset_name?: string | null;
  schedule_date: string;
  planned_amount: string;
  booked_amount: string;
  status: "Booked" | "Booked at Disposal" | "Due" | "Planned" | string;
  posting_error?: string | null;
  journal_entry?: string | null;
}

/** Maintenance due, from the report. */
export interface DueRow {
  asset?: string | null;
  asset_id: string;
  asset_name?: string | null;
  asset_maintenance: string;
  task_name?: string | null;
  next_due_date: string;
  days_overdue: number;
  status: string;
  assign_to?: string | null;
}

export interface Named {
  id: string;
  _titles?: Record<string, string>;
}
