import { Button, Link } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import {
  authorizeMcpServer,
  formatApiError,
  isApiError,
  isStaleOrAborted,
  type OutputMcpServerResponse,
} from "../../api/index.ts";
import { ConfirmDialog } from "../../components/confirm-dialog.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { text } from "../../i18n/locale.ts";
import { formatTimestamp } from "../format.ts";

interface AuthorizeLink {
  url: string;
  expiresIn: number;
  deadline: string;
}

/**
 * 授权码服务行上的「授权」和「撤销授权」。
 * 授权 URL 含一次性 state，只放进链接的 href，不用 window.open，避免被当成弹窗拦截，也不进地址栏。
 */
export function OAuthActions({
  server,
  onDeauthorize,
}: {
  server: OutputMcpServerResponse;
  onDeauthorize: (id: string) => void;
}) {
  const oauth = server.oauth;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [start, setStart] = useState<AuthorizeLink | null>(null);
  const lock = useRef(false);

  if (oauth === null || oauth.grant !== "authorization_code") {
    return null;
  }
  const needsAuthorization = oauth.status === "authorization_required";
  const authorized = oauth.status === "authorized";
  if (!needsAuthorization && !authorized) {
    return null;
  }

  async function onAuthorize() {
    if (lock.current) {
      return;
    }
    lock.current = true;
    setPending(true);
    setError(null);
    try {
      const result = await authorizeMcpServer(server.id);
      setStart({
        url: result.authorization_url,
        expiresIn: result.expires_in,
        deadline: new Date(Date.now() + result.expires_in * 1000).toISOString(),
      });
    } catch (caught) {
      setStart(null);
      if (!isStaleOrAborted(caught)) {
        setError(isApiError(caught) ? formatApiError(caught) : text("api.mcp.authorizeInvalid"));
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  const link = needsAuthorization && start !== null ? start : null;
  return (
    <>
      {needsAuthorization ? (
        <Button type="button" disabled={pending} onClick={() => void onAuthorize()}>
          {pending ? text("mcp.oauth.pending") : text("mcp.oauth.start")}
        </Button>
      ) : (
        <ConfirmDialog
          label={text("mcp.oauth.revoke")}
          title={text("mcp.oauth.revokeTitle")}
          description={text("mcp.oauth.revokeDescription")}
          resource={server.id}
          confirmLabel={text("mcp.oauth.revokeConfirm")}
          onConfirm={() => onDeauthorize(server.id)}
        />
      )}
      {link === null && error === null ? null : (
        <div style={{ flexBasis: "100%" }}>
          {link === null ? null : (
            <p className="hint">
              <Link href={link.url} target="_blank" rel="noopener noreferrer">
                {text("mcp.oauth.open")}
                <Link.ExternalIcon />
              </Link>{" "}
              {text("mcp.oauth.deadline", {
                deadline: formatTimestamp(link.deadline) || link.deadline,
                seconds: String(link.expiresIn),
              })}
            </p>
          )}
          {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
        </div>
      )}
    </>
  );
}
