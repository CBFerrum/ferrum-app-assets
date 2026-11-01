// An asset's maintenance plan (its tasks and when each falls due next), and the work open against it.

import { useQuery } from "@tanstack/react-query";
import { api, docPath } from "@ferrum/client";
import { list } from "./data";

export interface Task {
  id: string;
  task_name: string;
  maintenance_type?: string | null;
  periodicity?: string | null;
  assign_to?: string | null;
  last_completion_date?: string | null;
  next_due_date?: string | null;
}

export interface Plan {
  id: string;
  version?: number;
  tasks: Task[];
}

export interface OpenLog {
  id: string;
  task?: string | null;
  task_name?: string | null;
  due_date?: string | null;
  maintenance_status: string;
  completion_date?: string | null;
  assigned_to?: string | null;
  docstatus?: number;
}

export interface Fault {
  id: string;
  failure_date: string;
  repair_status: string;
  description?: string | null;
  docstatus?: number;
}

/** The plan of an asset (it has at most one), or null when it has none. */
export function usePlan(asset: string) {
  return useQuery({
    queryKey: ["assets", "plan-of", asset],
    queryFn: async ({ signal }) => {
      const found = await list<{ id: string }>("AssetMaintenance", { filters: [["asset", "=", asset]], fields: "asset", limit: 1 }, signal);
      return found.rows[0] ? api.get<Plan>(docPath("AssetMaintenance", found.rows[0].id), undefined, signal) : null;
    },
  });
}

/** The logs of work planned or under way, and the repairs not yet done. */
export function useOpenWork(asset: string) {
  return useQuery({
    queryKey: ["assets", "open-work", asset],
    queryFn: async ({ signal }) => {
      const [logs, faults] = await Promise.all([
        list<OpenLog>("AssetMaintenanceLog", { filters: [["asset", "=", asset], ["docstatus", "=", 0], ["maintenance_status", "in", ["Planned", "In Progress"]]], fields: "task,task_name,due_date,maintenance_status,assigned_to", sort: "due_date asc", limit: 50 }, signal).then((r) => r.rows),
        list<Fault>("AssetRepair", { filters: [["asset", "=", asset], ["docstatus", "=", 0], ["repair_status", "=", "Pending"]], fields: "failure_date,repair_status,description", sort: "failure_date asc", limit: 50 }, signal).then((r) => r.rows),
      ]);
      return { logs, faults };
    },
  });
}
