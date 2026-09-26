import { BrowserRouter } from "react-router";
import { AuthGate } from "./auth-gate.tsx";
import { SessionProvider } from "./session-context.tsx";

export function App() {
  return (
    <BrowserRouter>
      <SessionProvider>
        <AuthGate />
      </SessionProvider>
    </BrowserRouter>
  );
}
