// Asking the server: lists of records, reports, and the actions on one record. Every function takes
// what to ask and gives back what was answered (the screens show the server's words when it refuses).

import { api, docPath } from "@ferrum/client";
import type { Report } from "./types";

type Query = Record<string, string | number | boolean | undefined>;

export interface Rows<T> {
  rows: T[];
  total?: number | null;
}

export interface ListQuery {
  /** The API's own `[["field", "op", value]]`. */
  filters?: unknown[];
  fields?: string;
  sort?: string;
  search?: string;
  limit?: number;
  count?: boolean;
  titles?: boolean;
}

/** A page of records of an entity. */
export function list<T>(entity: string, query: ListQuery, signal?: AbortSignal) {
  const { filters, ...rest } = query;
  return api.get<Rows<T>>(docPath(entity), { ...rest, filters: filters?.length ? JSON.stringify(filters) : undefined }, signal);
}

export function report<T>(name: string, params: Query, signal?: AbortSignal) {
  return api.get<Report<T>>(`/api/reports/${encodeURIComponent(name)}`, params, signal);
}

/** Makes a document and submits it in one go (the server does both, or neither). */
export function create<T = { id: string }>(entity: string, body: unknown, key: string) {
  return api.post<T>(`${docPath(entity)}?submit=true`, body, { idempotencyKey: key });
}

/** Makes a draft. */
export function draft<T = { id: string }>(entity: string, body: unknown, key: string) {
  return api.post<T>(docPath(entity), body, { idempotencyKey: key });
}

export function action<T = unknown>(entity: string, id: string, name: string, body: unknown = {}, version?: number) {
  return api.post<T>(docPath(entity, id, "actions", name), body, { ifMatch: version });
}
