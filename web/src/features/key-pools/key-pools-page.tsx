import { Badge, Table } from "@cloudflare/kumo";
import { listKeyPools, type OutputKeyRuntimeStateResponse } from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";
import { formatCount, formatOptionalNumber } from "../format.ts";

const stateBadges = {
  available: "success",
  leased: "neutral",
  cooling: "warning",
} as const satisfies Record<OutputKeyRuntimeStateResponse, string>;

export function KeyPoolsPage() {
  const load = usePageLoad((signal) => listKeyPools({ signal }), []);

  return (
    <section className="page">
      <h2>{text("shell.routes.keyPools")}</h2>
      {load.loading && load.data === undefined ? <LoadingState /> : null}
      {load.error !== null && load.data === undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
      {load.data !== undefined && load.data.length === 0 ? (
        <EmptyState
          title={text("keyPools.empty")}
          description={text("keyPools.emptyDescription")}
        />
      ) : null}
      {load.data?.map((pool) => (
        <section key={pool.resource_id}>
          <h3 className="section-title">
            {pool.resource_id} <span className="hint">· {pool.strategy}</span>
          </h3>
          <div className="tablewrap">
            <Table>
              <Table.Header variant="compact">
                <Table.Row>
                  <Table.Head>{text("keyPools.columns.id")}</Table.Head>
                  <Table.Head>{text("keyPools.columns.state")}</Table.Head>
                  <Table.Head>{text("keyPools.columns.leased")}</Table.Head>
                  <Table.Head>{text("keyPools.columns.cooling")}</Table.Head>
                  <Table.Head>{text("keyPools.columns.weight")}</Table.Head>
                  <Table.Head>{text("keyPools.columns.latency")}</Table.Head>
                  <Table.Head>{text("keyPools.columns.ref")}</Table.Head>
                </Table.Row>
              </Table.Header>
              <Table.Body>
                {pool.keys.map((key) => (
                  <Table.Row key={key.key_id}>
                    <Table.Cell>{key.key_id}</Table.Cell>
                    <Table.Cell>
                      <Badge variant={stateBadges[key.state]} appearance="dot">
                        {key.state}
                      </Badge>
                    </Table.Cell>
                    <Table.Cell>{formatCount(key.leased_count)}</Table.Cell>
                    <Table.Cell>{formatOptionalNumber(key.cooling_remaining_ms)}</Table.Cell>
                    <Table.Cell>{formatCount(key.weight)}</Table.Cell>
                    <Table.Cell>{formatOptionalNumber(key.ewma_latency_ms)}</Table.Cell>
                    <Table.Cell>{key.ref}</Table.Cell>
                  </Table.Row>
                ))}
              </Table.Body>
            </Table>
          </div>
        </section>
      ))}
      {load.error !== null && load.data !== undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
    </section>
  );
}
