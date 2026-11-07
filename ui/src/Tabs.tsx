// Assets' own screens, at the top of each.

import { NavLink } from "react-router";
import { t } from "@ferrum/client";

export function Tabs() {
  return (
    <nav className="ast-tabs" aria-label={t("Assets")}>
      <NavLink to="/" end>
        {t("Overview")}
      </NavLink>
      <NavLink to="/verify">{t("Verify")}</NavLink>
    </nav>
  );
}
