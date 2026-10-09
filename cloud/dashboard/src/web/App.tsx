import { NavLink, Route, Routes } from "react-router";
import { SessionDetailPage } from "./pages/SessionDetail";
import { SessionsPage } from "./pages/Sessions";
import { TotalsPage } from "./pages/Totals";

export function App() {
  return (
    <>
      <header className="top">
        <strong>claude-multi</strong>
        <nav>
          <NavLink to="/" end>Sessions</NavLink>
          <NavLink to="/totals">Totals</NavLink>
        </nav>
      </header>
      <main>
        <Routes>
          <Route path="/" element={<SessionsPage />} />
          <Route path="/sessions/:id" element={<SessionDetailPage />} />
          <Route path="/totals" element={<TotalsPage />} />
          <Route path="*" element={<p className="muted">Not found.</p>} />
        </Routes>
      </main>
    </>
  );
}
