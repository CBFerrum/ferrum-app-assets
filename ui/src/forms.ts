// What every dialog does when it asks the server to do something: wait, and then either close or
// show the server's reason, against the field it is about when it names one.

import { useCallback, useState } from "react";
import { ApiError, errorMessage } from "@ferrum/client";

export interface Act {
  busy: boolean;
  error: unknown;
  /** Runs the request; true when it went through. */
  run: (request: () => Promise<unknown>) => Promise<boolean>;
  /** What the server said about one field. */
  field: (name: string) => string | undefined;
  /** What it said that no field of this dialog shows. */
  general: (fields: string[]) => string | null;
}

export function useAct(): Act {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const run = useCallback(async (request: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await request();
      return true;
    } catch (e) {
      setError(e);
      return false;
    } finally {
      setBusy(false);
    }
  }, []);
  const field = (name: string) => (error instanceof ApiError ? error.fields.find((f) => f.field === name)?.message : undefined);
  const general = (fields: string[]) => {
    if (!error) return null;
    if (error instanceof ApiError && error.fields.length && error.fields.every((f) => fields.includes(f.field))) return null;
    return errorMessage(error);
  };
  return { busy, error, run, field, general };
}
