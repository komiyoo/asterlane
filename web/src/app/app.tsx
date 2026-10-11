import { LinkProvider } from "@cloudflare/kumo";
import { BrowserRouter } from "react-router";
import { AppLink } from "./app-link.tsx";
import { AuthGate } from "./auth-gate.tsx";
import { SessionProvider } from "./session-context.tsx";

export function App() {
  return (
    <BrowserRouter>
      <LinkProvider component={AppLink}>
        <SessionProvider>
          <AuthGate />
        </SessionProvider>
      </LinkProvider>
    </BrowserRouter>
  );
}
