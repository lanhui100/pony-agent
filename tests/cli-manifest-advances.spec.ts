import { describe, it, expect } from "vitest";
import {
  evaluateCliAdvance,
  parseCliVersion
} from "../scripts/check-cli-manifest-advances.mjs";

/**
 * 契约（2026-11-08 遗留项）：发布侧必须拦住「清单 version 未递增」——
 * 否则客户端 `pony update` 永远判定「已是最新」，新的 CLI 构建永远不会被下载，
 * 且客户端侧无任何报错可看（现场实证：v0.1.185 清单 version=0.1.158 与安装版相同）。
 */
describe("check-cli-manifest-advances", () => {
  describe("parseCliVersion", () => {
    it("接受 X.Y.Z 与 v/V 前缀，拒绝非三段式（不补零、不猜测）", () => {
      expect(parseCliVersion("0.1.159")).toEqual([0, 1, 159]);
      expect(parseCliVersion("v0.1.159")).toEqual([0, 1, 159]);
      expect(parseCliVersion("V1.2.3")).toEqual([1, 2, 3]);
      for (const bad of ["0.1", "0.1.159.1", "", "a.b.c", "0.1.-3", null, undefined]) {
        expect(parseCliVersion(bad as unknown as string)).toBeNull();
      }
    });
  });

  describe("evaluateCliAdvance", () => {
    it("严格递增才通过（相等即失败：相等正是「永远已是最新」的病根）", () => {
      expect(evaluateCliAdvance("0.1.159", "0.1.158").ok).toBe(true);
      expect(evaluateCliAdvance("0.2.0", "0.1.999").ok).toBe(true);

      const same = evaluateCliAdvance("0.1.158", "0.1.158");
      expect(same.ok).toBe(false);
      expect(same.reason).toContain("已是最新");
      expect(same.reason).toContain("--target core");
    });

    it("回退版本必须失败（回退的 CLI 构建客户端发现不到）", () => {
      const verdict = evaluateCliAdvance("0.1.157", "0.1.158");
      expect(verdict.ok).toBe(false);
      expect(verdict.reason).toContain("低于上游清单版本");
    });

    it("上游无历史清单时放行（首个清单），但上游版本非法必须 fail-closed", () => {
      expect(evaluateCliAdvance("0.1.1", "none").ok).toBe(true);
      expect(evaluateCliAdvance("0.1.1", null).ok).toBe(true);
      expect(evaluateCliAdvance("0.1.1", "").ok).toBe(true);

      const broken = evaluateCliAdvance("0.1.1", "0.1");
      expect(broken.ok).toBe(false);
      expect(broken.reason).toContain("fail-closed");
    });

    it("本轮版本非法必须失败（不能用空版本发布清单）", () => {
      const verdict = evaluateCliAdvance("", "0.1.158");
      expect(verdict.ok).toBe(false);
      expect(verdict.reason).toContain("本轮 CLI 版本非法");
    });
  });
});
