import { expect, test } from "vite-plus/test";
import { commitIfCurrent, createLatestGate } from "./latest.ts";

test("a slower query does not replace a newer result", async () => {
  const gate = createLatestGate();
  let applied = "";
  let releaseSlow: (value: string) => void = () => undefined;
  const slow = new Promise<string>((resolve) => {
    releaseSlow = resolve;
  });
  const slowId = gate.next();
  const slowTask = slow.then((value) =>
    commitIfCurrent(gate, slowId, new AbortController().signal, value, (next) => {
      applied = next;
    }),
  );

  const fastId = gate.next();
  commitIfCurrent(gate, fastId, new AbortController().signal, "fast", (next) => {
    applied = next;
  });
  expect(applied).toBe("fast");

  releaseSlow("slow");
  await slowTask;
  expect(applied).toBe("fast");
});

test("switching pages drops the previous load", async () => {
  const gate = createLatestGate();
  const first = new AbortController();
  const firstId = gate.next();
  let applied = "";
  let release: (value: string) => void = () => undefined;
  const pending = new Promise<string>((resolve) => {
    release = resolve;
  }).then((value) =>
    commitIfCurrent(gate, firstId, first.signal, value, (next) => {
      applied = next;
    }),
  );

  first.abort();
  const secondId = gate.next();
  commitIfCurrent(gate, secondId, new AbortController().signal, "usage", (next) => {
    applied = next;
  });
  release("events");
  await pending;
  expect(applied).toBe("usage");
});
