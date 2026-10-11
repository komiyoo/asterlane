import type { OutputRequestEventResponse } from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";
import { formatOptionalNumber } from "../format.ts";
import { SaveDefaultControl } from "../tools/save-default-control.tsx";

export function EventDetail({ event }: { event: OutputRequestEventResponse }) {
  const missing = text("events.detail.missing");
  const latency = formatOptionalNumber(event.upstream_latency_ms) || text("common.dash");
  return (
    <div className="detail">
      <p>{text("events.detail.upstream", { value: latency })}</p>
      <p>{text("events.detail.args")}</p>
      <pre>{event.request_args ?? missing}</pre>
      <p>{text("events.detail.response")}</p>
      <pre>{event.response_preview ?? missing}</pre>
      {/* 默认调用参数只对工具有意义；prompt 与 resource 的名字不在工具目录里 */}
      {event.request_kind === "tool" ? (
        <SaveDefaultControl toolName={event.tool_name} requestArgs={event.request_args} />
      ) : null}
    </div>
  );
}
