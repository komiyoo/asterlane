import type { OutputRequestEventResponse } from "../../api/index.ts";
import { formatOptionalNumber } from "../format.ts";
import { SaveDefaultControl } from "../tools/save-default-control.tsx";

export function EventDetail({ event }: { event: OutputRequestEventResponse }) {
  return (
    <div className="detail">
      <p>上游延迟(ms)：{formatOptionalNumber(event.upstream_latency_ms) || "—"}</p>
      <p>请求参数</p>
      <pre>{event.request_args ?? "（未捕获）"}</pre>
      <p>响应预览</p>
      <pre>{event.response_preview ?? "（未捕获）"}</pre>
      {/* 默认调用参数只对工具有意义；prompt 与 resource 的名字不在工具目录里 */}
      {event.request_kind === "tool" ? (
        <SaveDefaultControl toolName={event.tool_name} requestArgs={event.request_args} />
      ) : null}
    </div>
  );
}
