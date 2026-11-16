// The Rust root tested from TypeScript: `nsh test` runs this file in a
// headless runner built with the crate, and `open(".")` starts the root
// (`nsh.json` "root"). With `--target tui` the same test runs through the
// terminal host in an in-memory terminal, which is how CI checks the TUI.
//
// No network is stubbed: the shell must render whether or not the API is
// reachable, and these assertions only cover the shell and the panels.

import { expect, open, test } from "nsh/test";

test("the dashboard renders its shell and panels", async () => {
  const app = await open(".", { width: 1280, height: 900 });
  await app.waitFor("text:氧化层 · oxide-fab");
  await app.waitFor("text:四条工序队列");
  await app.waitFor("text:机台灯号");
  await app.waitFor("text:批次与逐片追溯");
  await app.waitFor("text:SPC · 量测控制图");
  await app.waitFor("text:一致性 · outbox 积压与追平");
  expect(app.texts().length).toBeGreaterThan(5);
});
