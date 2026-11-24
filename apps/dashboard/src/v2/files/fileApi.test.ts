import { expect, it, vi } from "vitest";
import type { request } from "../../api";
import { FileApi } from "./fileApi";
const project = `p_${"a".repeat(32)}`,
  root = `r_${"a".repeat(32)}`;
function controlled() {
  const completions: Array<() => void> = [];
  const call = vi.fn((...input: Parameters<typeof request>) => {
    if (!input[0]) throw new Error("Missing request path");
    return new Promise((resolve) => completions.push(() => resolve({})));
  });
  return { call, api: new FileApi(call as typeof request), completions };
}
it("bounds directory I/O and prioritizes a user save without changing its project/root", async () => {
  const f = controlled();
  const reads = Array.from({ length: 5 }, (_, index) =>
    f.api.tree(project, root, `directory-${index}`, false)
  );
  const file = { project_id: project, root_id: root, path: "src/main.rs" };
  const saved = f.api.save(file, "v_observed", "new content");
  expect(f.call).toHaveBeenCalledTimes(2);
  f.completions.shift()!();
  await vi.waitFor(() => expect(f.call).toHaveBeenCalledTimes(3));
  expect(f.call.mock.calls[2][0]).toBe(`/projects/${project}/files/content`);
  expect(JSON.parse((f.call.mock.calls[2][1] as RequestInit).body as string).file).toEqual(file);
  for (let turn = 0; turn < 8; turn++) {
    for (const finish of f.completions.splice(0)) finish();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  await Promise.all([...reads, saved]);
});
it("sends cancellation without waiting behind slow file jobs", async () => {
  const f = controlled();
  const first = f.api.tree(project, root, "one", false),
    second = f.api.tree(project, root, "two", false);
  const cancelled = f.api.cancel(project, `q_${"b".repeat(32)}`);
  expect(f.call).toHaveBeenCalledTimes(3);
  for (const finish of f.completions) finish();
  await Promise.all([first, second, cancelled]);
});
it("does not transmit an aborted queued search", async () => {
  const f = controlled();
  const first = f.api.tree(project, root, "one", false),
    second = f.api.tree(project, root, "two", false),
    controller = new AbortController();
  const search = f.api.search(
    project,
    {
      id: `q_${"b".repeat(32)}`,
      root_id: root,
      query: "query",
      content: true,
      case_sensitive: false,
      regex: false,
      include_hidden: false,
      path_filter: "",
    },
    controller.signal
  );
  const assertion = expect(search).rejects.toMatchObject({ name: "AbortError" });
  controller.abort();
  for (const finish of f.completions) finish();
  await Promise.all([first, second, assertion]);
  expect(f.call).toHaveBeenCalledTimes(2);
});
