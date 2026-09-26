import { listKeyPools } from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { formatOptionalNumber } from "../format.ts";

export function KeyPoolsPage() {
  const load = usePageLoad((signal) => listKeyPools({ signal }), []);

  return (
    <section className="page">
      <h2>密钥池</h2>
      {load.loading && load.data === undefined ? <LoadingState /> : null}
      {load.error !== null && load.data === undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
      {load.data !== undefined && load.data.length === 0 ? (
        <EmptyState title="未配置密钥池" description="资源级 key_pool 尚未配置。" />
      ) : null}
      {load.data?.map((pool) => (
        <section key={pool.resource_id}>
          <h3>
            {pool.resource_id} <span className="hint">· {pool.strategy}</span>
          </h3>
          <div className="tablewrap">
            <table>
              <thead>
                <tr>
                  <th>密钥 ID</th>
                  <th>状态</th>
                  <th>租借数</th>
                  <th>冷却剩余(ms)</th>
                  <th>权重</th>
                  <th>加权延迟(ms)</th>
                  <th>引用</th>
                </tr>
              </thead>
              <tbody>
                {pool.keys.map((key) => (
                  <tr key={key.key_id}>
                    <td>{key.key_id}</td>
                    <td>{key.state}</td>
                    <td>{key.leased_count}</td>
                    <td>{formatOptionalNumber(key.cooling_remaining_ms)}</td>
                    <td>{key.weight}</td>
                    <td>{formatOptionalNumber(key.ewma_latency_ms)}</td>
                    <td>{key.ref}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ))}
      {load.error !== null && load.data !== undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
    </section>
  );
}
