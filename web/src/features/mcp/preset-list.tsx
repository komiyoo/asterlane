import { Button } from "@cloudflare/kumo";
import type { OutputMcpPresetResponse } from "../../api/index.ts";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";

export function PresetList({
  presets,
  error,
  loading,
  notice,
  enablingId,
  onEnable,
  onConfigure,
  onRetry,
}: {
  presets: OutputMcpPresetResponse[] | null;
  error: unknown;
  loading: boolean;
  notice: string | null;
  enablingId: string | null;
  onEnable: (preset: OutputMcpPresetResponse) => void;
  onConfigure: (preset: OutputMcpPresetResponse) => void;
  onRetry: () => void;
}) {
  return (
    <section aria-label="内置集成" className="card">
      <h3>内置集成</h3>
      {notice === null ? null : <FormMessage tone="error">{notice}</FormMessage>}
      {error !== null ? <ErrorState error={error} onRetry={onRetry} /> : null}
      {loading && presets === null ? <LoadingState /> : null}
      {presets === null ? null : (
        <div className="tablewrap">
          <table>
            <thead>
              <tr>
                <th>集成</th>
                <th>描述</th>
                <th>状态</th>
                <th>凭据</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {presets.length === 0 ? (
                <tr>
                  <td colSpan={5}>无内置集成</td>
                </tr>
              ) : (
                presets.map((preset) => (
                  <tr key={preset.id}>
                    <td>
                      {preset.id}
                      <div className="hint">
                        {preset.domain} / {preset.provider}
                      </div>
                    </td>
                    <td style={{ whiteSpace: "normal" }}>{preset.description}</td>
                    <td>{preset.enabled ? "已启用" : "未启用"}</td>
                    <td>
                      {preset.requires_key ? "需 key" : "免费"}
                      {safeApplyUrl(preset.apply_url) === null ? null : (
                        <>
                          {" "}
                          <a
                            href={safeApplyUrl(preset.apply_url) ?? undefined}
                            target="_blank"
                            rel="noreferrer"
                          >
                            申请 key
                          </a>
                        </>
                      )}
                    </td>
                    <td>
                      {preset.enabled ? (
                        <Button type="button" disabled>
                          已启用
                        </Button>
                      ) : preset.requires_key ? (
                        <Button type="button" onClick={() => onConfigure(preset)}>
                          配置 key 启用
                        </Button>
                      ) : (
                        <Button
                          type="button"
                          disabled={enablingId === preset.id}
                          onClick={() => onEnable(preset)}
                        >
                          {enablingId === preset.id ? "启用中…" : "启用"}
                        </Button>
                      )}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function safeApplyUrl(value: string | null): string | null {
  if (value === null) {
    return null;
  }
  if (value.startsWith("https://") || value.startsWith("http://")) {
    return value;
  }
  return null;
}
