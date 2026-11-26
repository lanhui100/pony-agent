import assert from "node:assert/strict";
import path from "node:path";
import { buildLayerSteps, buildRustTestStep, main, prepareLayerEnv } from "./run-test-layer.mjs";

// 1. nextest 未安装时自动剥离 --profile / -E 并降级为 cargo test
{
  const step = buildRustTestStep({
    profile: "quick",
    extraArgs: ["-p", "pony-agent-core", "--lib", "--profile", "ci", "-E", "test(foo)"],
    hasNextest: false,
  });
  assert.equal(step.command, "node");
  assert.deepEqual(step.args, [
    path.join("scripts", "invoke-rust-target.mjs"),
    "test",
    "test",
    "-p",
    "pony-agent-core",
    "--lib",
  ]);
}

// 2. nextest 已安装时保留 profile 与过滤参数
{
  const step = buildRustTestStep({
    profile: "quick",
    extraArgs: ["-p", "pony-agent-core", "--lib"],
    hasNextest: true,
  });
  assert.deepEqual(step.args, [
    path.join("scripts", "invoke-rust-target.mjs"),
    "test",
    "nextest",
    "run",
    "--profile",
    "quick",
    "-p",
    "pony-agent-core",
    "--lib",
  ]);
}

// 3. L0-L3 分层矩阵完整性
{
  const l0 = buildLayerSteps("l0", [], false);
  assert.deepEqual(l0[0].args, ["run", "check:fast"]);

  const l1 = buildLayerSteps("l1", ["--no-run"], false);
  assert.equal(l1.length, 2);
  assert.ok(l1[0].args.includes("--workspace"));
  assert.ok(l1[0].args.includes("pony-agent"));
  assert.equal(l1[1].args[1], "typecheck");

  const l2 = buildLayerSteps("l2", [], true);
  assert.equal(l2.length, 4);
  assert.deepEqual(l2[0].args, ["run", "test:unit"]);
  assert.deepEqual(l2[1].args, ["run", "rust:lint"]);
  assert.deepEqual(l2[2].args, ["run", "rust:lint:tauri"]);
  assert.ok(l2[3].args.includes("--workspace"));

  const l3 = buildLayerSteps("l3", [], false);
  assert.equal(l3.length, 3);
  assert.ok(l3[0].args.includes("--workspace"));
  assert.ok(l3[0].args.includes("--no-fail-fast"));
  assert.deepEqual(l3[2].args, ["run", "build"]);
}

// 4. 非法层级参数非零退出
{
  const code = main(["bogus-layer"]);
  assert.equal(code, 1);
}

// 5. 非 l0 层级自动注入 PONY_AGENT_DATA_DIR，不覆盖外部值，l0 与 dryRun 不新建目录
{
  const prepared = prepareLayerEnv({}, "l1", {
    dryRun: false,
    createTempDir: () => "/tmp/mock-isolated-data",
  });
  assert.equal(prepared.autoDataDir, "/tmp/mock-isolated-data");
  assert.equal(prepared.env.PONY_AGENT_DATA_DIR, "/tmp/mock-isolated-data");

  const preserved = prepareLayerEnv({ PONY_AGENT_DATA_DIR: "/tmp/user-dir" }, "l3", {
    dryRun: false,
    createTempDir: () => {
      throw new Error("should not allocate temp dir when already set");
    },
  });
  assert.equal(preserved.autoDataDir, null);
  assert.equal(preserved.env.PONY_AGENT_DATA_DIR, "/tmp/user-dir");

  const l0Env = prepareLayerEnv({}, "l0", {
    dryRun: false,
    createTempDir: () => {
      throw new Error("should not allocate temp dir for l0");
    },
  });
  assert.equal(l0Env.autoDataDir, null);
}

console.log("run-test-layer.spec: PASS");
