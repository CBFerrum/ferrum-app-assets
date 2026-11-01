// Assets' own screens, at the top of each.

import { NavLink } from "react-router";

export function Tabs() {
  return (
    <nav className="ast-tabs" aria-label="Assets">
      <NavLink to="/" end>
        Overview
      </NavLink>
      <NavLink to="/verify">Verify</NavLink>
    </nav>
  );
}
