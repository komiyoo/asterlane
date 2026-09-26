import { BrowserRouter, Route, Routes } from "react-router";
import { SmokePage } from "./smoke-page.tsx";

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<SmokePage />} />
      </Routes>
    </BrowserRouter>
  );
}
