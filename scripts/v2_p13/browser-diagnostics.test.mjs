import assert from "node:assert/strict";
import http from "node:http";
import { once } from "node:events";
import test from "node:test";
import { browserFailure, requireHttpOk } from "./browser-diagnostics.mjs";

test("PRIV-07 authenticated error projects status alone", async () => {
  const payload = "synthetic-private-response-body";
  const credential = "synthetic-test-credential-only";
  const server = http.createServer((request, response) => {
    response.writeHead(request.headers.authorization === credential ? 409 : 401);
    response.end(payload);
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  try {
    const raw = await fetch(`http://127.0.0.1:${server.address().port}/synthetic`, {
      headers: { authorization: credential },
    });
    const response = {
      ok: () => raw.ok,
      status: () => raw.status,
      text: () => assert.fail("Body access is forbidden on this failure path"),
    };
    let output;
    try {
      requireHttpOk(response);
      assert.fail("Non-success response must fail");
    } catch (error) {
      output = browserFailure(error);
    }
    assert.equal(raw.bodyUsed, false);
    assert.deepEqual(output, { code: "HTTP_STATUS", expected: "2xx", actual: 409 });
    const json = JSON.stringify(output);
    assert.ok(!json.includes(payload) && !json.includes(credential));
    assert.deepEqual(browserFailure(new Error(payload)), { code: "BROWSER_EXECUTION" });
  } finally {
    server.close();
    await once(server, "close");
  }
});
