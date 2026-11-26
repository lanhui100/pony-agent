import { execSync } from "node:child_process";
import path from "node:path";
import fs from "node:fs";

const REPO_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const SCRIPT = path.join(REPO_ROOT, "scripts", "check-bin-test-imports.mjs");

console.log("=== 运行 check-bin-test-imports.spec.mjs ===");

// 1. 基线正向测试：当前代码库应 PASS
try {
  const out = execSync(`node ${SCRIPT}`, { cwd: REPO_ROOT, encoding: "utf8" });
  console.log("PASS 1: 当前仓库基线状态正常 Pass");
} catch (e) {
  console.error("FAIL 1: 当前仓库基线状态未 Pass", e.message);
  process.exit(1);
}

// 2. 负向测试：模拟新增一个未在基线中的测试文件，断言必须 FAIL 且 Exit 1
const fakeTest = path.join(REPO_ROOT, "crates", "pony-agent-core", "tests", "fake_bad_test.rs");
try {
  fs.writeFileSync(fakeTest, '#[path = "../src/bin/tui.rs"]\nmod tui;\n', "utf8");
  try {
    execSync(`node ${SCRIPT}`, { cwd: REPO_ROOT, encoding: "utf8" });
    console.error("FAIL 2: 未拦截新增的违规引入！");
    process.exit(1);
  } catch (e) {
    if (e.status === 1 && e.stderr.includes("架构门禁：检测到未在基线中的测试文件")) {
      console.log("PASS 2: 成功拦截未登记的新增违规文件（Exit 1 + 报错信息清晰）");
    } else {
      console.error("FAIL 2: 退出码或错误信息不符合预期:", e.stderr);
      process.exit(1);
    }
  }
} finally {
  if (fs.existsSync(fakeTest)) {
    fs.unlinkSync(fakeTest);
  }
}

console.log("check-bin-test-imports.spec: ALL PASSED");