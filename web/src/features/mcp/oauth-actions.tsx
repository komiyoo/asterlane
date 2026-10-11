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
        setError(isApiError(caught) ? formatApiError(caught) : "授权响应无效");
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
          {pending ? "授权中…" : "授权"}
        </Button>
      ) : (
        <ConfirmDialog
          label="撤销授权"
          title="撤销授权"
          description="清除网关保存的凭据，并让当前连接失效。不会通知授权服务器吊销 token。"
          resource={server.id}
          confirmLabel="确认撤销"
          onConfirm={() => onDeauthorize(server.id)}
        />
      )}
      {link === null && error === null ? null : (
        <div style={{ flexBasis: "100%" }}>
          {link === null ? null : (
            <p className="hint">
              <Link href={link.url} target="_blank" rel="noopener noreferrer">
                打开授权页
                <Link.ExternalIcon />
              </Link>{" "}
              有效期至 {link.deadline}（{link.expiresIn} 秒）
            </p>
          )}
          {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
        </div>
      )}
    </>
  );
}
