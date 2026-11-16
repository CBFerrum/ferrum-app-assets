// Assets' own interface (Ferrum docs/plan/apps-ui.md §13) at /assets/: an overview of what the company
// owns, what it is worth and what needs a person, an asset's page at /a/{id}, walking a room to verify
// what is there at /verify, and the register's records (assets, categories, places, movements,
// revaluations, repairs, maintenance, disposals) as lists and forms in the records frame, with
// Assets' modules in the sidebar under the platform's bar. Records open through /open, in the app that
// shows them: an asset here, its purchase invoice or journal entries in ERP.

import { useEffect } from "react";
import { Link, useRouteError } from "react-router";
import { shell, t } from "@ferrum/client";
import { StatePanel } from "@ferrum/kit";
import { mountApp, RequireAuth, setFrameMode } from "@ferrum/records/frame";
import "./styles.css";

/** A page of the records frame, loaded when first opened. */
const page = (name: "FormView" | "ListView" | "ReportView") => () =>
  import("./records").then((m) => ({ Component: m[name] }));

/** Assets' packages: the sidebar shows their modules. */
const ASSET_PACKAGES = ["assets", "assets_ops"];

function RouteError() {
  return <StatePanel kind="failed" error={useRouteError()} onRetry={() => window.location.reload()} />;
}

function NotFound() {
  return (
    <StatePanel kind="empty" title={t("Page not found")} action={<Link to="/">{t("Go to the assets overview")}</Link>} />
  );
}

/** Every page tells the shell it started. */
function Started() {
  useEffect(() => shell().ready(), []);
  return null;
}

setFrameMode("app", ASSET_PACKAGES);

mountApp({
  route: "assets",
  catalogs: { ar: () => import("./locales/ar.json"), hi: () => import("./locales/hi.json") },
  routes: [
    {
      path: "/",
      element: (
        <>
          <Started />
          <RequireAuth />
        </>
      ),
      errorElement: <RouteError />,
      children: [
        { index: true, lazy: () => import("./Home").then((m) => ({ Component: m.Home })) },
        { path: "verify", lazy: () => import("./Verify").then((m) => ({ Component: m.Verify })) },
        { path: "a/:id", lazy: () => import("./Asset").then((m) => ({ Component: m.AssetRoute })) },
        { path: "e/:entity", lazy: page("ListView") },
        { path: "e/:entity/:id", lazy: page("FormView") },
        { path: "r/:name", lazy: page("ReportView") },
        { path: "*", element: <NotFound /> },
      ],
    },
  ],
});
