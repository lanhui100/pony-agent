/**
 * wave-pds-3 · **V13：纯文本快路径的尾巴**（契约 v8）—— jsdom
 *
 * 背景：`WorkspaceTurnItem.vue` 里 `isSimpleTextContent` 命中时会走**纯文本快路径**
 * （`<div class="markdown-body whitespace-pre-wrap">{{ event.content }}</div>`），完全不经过
 * `MarkdownRenderer` ⇒ 这类回复**永远拿不到尾巴**。而"纯段落正文"恰恰是 v7 判定里最该享受浮现的形态。
 *
 * v8 裁定：纯文本快路径也必须发尾巴（同一 `[data-stream-tail]` 标记、同一开关、同一降级规则）；
 * 切分 = 已释放文本中**最后一个空行（`\n\n`）之后**的部分（无空行则整段即尾巴）；
 * 遮罩同 v5 R-A4（单行才启用 `mask-image`，多行 `none`）。
 *
 * 本文件覆盖 V13-1 / V13-3 / V13-4 / V13-5（jsdom 可判定部分）。
 * **V13-2（mask-image 单行/多行）在 jsdom 无法判定**（CSSOM 不实现 mask-image 取值）⇒ 放在
 * `e2e/*.pw.ts` 的 `V13-2 [Chromium]`。
 *
 * 判据纪律：
 * - V13-4 用**全等**断言（`toBe`），不用 `includes` —— 纯文本路径最大的风险就是吞字/重字；
 * - 期望值一律由**源文本**现算（`splitMarkdownBlocks` 不适用于纯文本路径，改用 v8 的空行规则）；
 * - 每例都带"内容确实到达 DOM"的自检，防「什么都没渲染 ⇒ 尾巴自然没有」的假绿。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import {
  TAIL_LEN_ATTR,
  TAIL_SELECTOR,
  contentColumn,
  installDomStubs,
  mountWorkspace,
  patchAssistant,
  setRootReduceMotion,
  setTailSwitch,
  settleRender,
  stubReducedMotion
} from "./pds-streaming-harness";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

/** 纯文本正文承载节点（纯文本快路径与 markdown 路径共用 `.markdown-body`）。 */
function plainBody(wrapper: ReturnType<typeof mountWorkspace>): Element | null {
  return contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
}

/** v8 切分规则：已释放文本中最后一个空行（`\n\n`）之后的部分；无空行则整段即尾巴。 */
function expectedTail(releasedText: string): string {
  const index = releasedText.lastIndexOf("\n\n");
  return index >= 0 ? releasedText.slice(index + 2) : releasedText;
}

/** 典型纯文本流式内容：两段纯中文，**不含任何 markdown 标记**（否则会走 markdown 路径）。 */
const PLAIN_PARAGRAPH_ONE = "第一段纯文本内容，没有任何 markdown 标记，用于验证纯文本快路径同样能发尾巴。";
const PLAIN_PARAGRAPH_TWO = "第二段纯文本内容正在逐字输出，长度超过一行以便观察遮罩与折行行为。";
const PLAIN_FULL = `${PLAIN_PARAGRAPH_ONE}\n\n${PLAIN_PARAGRAPH_TWO}`;

async function mountPlainStreaming(content: string, status: "pending" | "done" = "pending") {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, content, status);
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  return { wrapper, store };
}

describe("wave-pds-3 V13：纯文本快路径的尾巴（契约 v8）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    // 释放速率放大 ⇒ 已释放文本 == 完整 content，使 V13-4 的全等断言有确定真值。
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setTailSwitch(null);
    setRootReduceMotion(false);
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    installDomStubs();
  });

  afterEach(() => {
    setRootReduceMotion(false);
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("V13-1: 纯文本流式（无 markdown 标记）⇒ 尾巴数量 = 1，data-tail-len = 源尾字符数", async () => {
    // Arrange
    const { wrapper } = await mountPlainStreaming(PLAIN_FULL);
    const body = plainBody(wrapper);
    const tailSource = expectedTail(PLAIN_FULL);

    // Assert 自检：内容确实渲染到 DOM（且不是走 markdown 路径 —— 该路径不含 HTML 标签）
    expect(body, "V13-1 自检：必须存在 .markdown-body 承载节点").not.toBeNull();
    expect(
      body?.querySelector("p, ul, ol, pre, table, h1, h2"),
      "V13-1 前置：本用例必须命中**纯文本快路径**（不得渲染出 markdown 块级元素）"
    ).toBeNull();

    // Assert ①：尾巴数量 = 1
    const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);
    expect(
      tails.length,
      `V13-1：纯文本快路径必须发且仅发 1 个 [data-stream-tail]；实际 ${tails.length}；`
        + `body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 400))}`
    ).toBe(1);

    // Assert ②：data-tail-len = 源尾字符数（v8 规则：最后一个空行之后）
    expect(
      tailSource.length,
      "V13-1 夹具自检：源尾必须非空（两段式内容 ⇒ 空行之后的第二段）"
    ).toBeGreaterThan(0);
    expect(
      tails[0]!.getAttribute(TAIL_LEN_ATTR),
      `V13-1：data-tail-len 必须等于源尾字符数 ${tailSource.length}（实际尾巴源=${JSON.stringify(tailSource)}）`
    ).toBe(String(tailSource.length));

    wrapper.unmount();
  });

  it("V13-3: 开关 off 与 reduced-motion 两路 ⇒ 尾巴数量 = 0，且可见文本与开启时逐字一致", async () => {
    // Arrange：先测「开启」基线文本
    setTailSwitch(null);
    stubReducedMotion(false);
    const on = await mountPlainStreaming(PLAIN_FULL);
    const onText = plainBody(on.wrapper)?.textContent ?? "";
    const onTailCount = plainBody(on.wrapper)?.querySelectorAll(TAIL_SELECTOR).length ?? 0;
    on.wrapper.unmount();

    // Assert 自检：开启态必须真的有尾巴（否则"关闭后也为 0"是必然的，用例无判别力）
    expect(
      onTailCount,
      `V13-3 自检：开启态必须渲染尾巴（实际 ${onTailCount}）—— 若为 0，本用例退化成「0 个尾巴不含尾巴」`
    ).toBe(1);
    expect(onText, "V13-3 自检：开启态可见文本必须非空").not.toBe("");

    // Act ①：开关 off
    setTailSwitch("off");
    const off = await mountPlainStreaming(PLAIN_FULL);
    const offBody = plainBody(off.wrapper);
    const offTailCount = offBody?.querySelectorAll(TAIL_SELECTOR).length ?? 0;
    const offText = offBody?.textContent ?? "";
    off.wrapper.unmount();

    // Assert ①
    expect(offTailCount, "V13-3：开关 off 时纯文本快路径不得渲染尾巴").toBe(0);
    expect(
      offText,
      `V13-3：开关 off 的可见文本必须与开启态逐字一致（防吞字）；`
        + `开启=${JSON.stringify(onText.slice(0, 120))}；关闭=${JSON.stringify(offText.slice(0, 120))}`
    ).toBe(onText);

    // Act ②：reduced-motion（系统偏好）
    setTailSwitch(null);
    stubReducedMotion(true);
    const reduced = await mountPlainStreaming(PLAIN_FULL);
    const reducedBody = plainBody(reduced.wrapper);
    const reducedTailCount = reducedBody?.querySelectorAll(TAIL_SELECTOR).length ?? 0;
    const reducedText = reducedBody?.textContent ?? "";
    reduced.wrapper.unmount();

    // Assert ②
    expect(reducedTailCount, "V13-3：reduced-motion 下纯文本快路径不得渲染尾巴").toBe(0);
    expect(
      reducedText,
      "V13-3：reduced-motion 的可见文本必须与开启态逐字一致（防吞字）"
    ).toBe(onText);

    // Act ③：应用内降级属性（祖先 data-pds-reduce-motion="true"）
    stubReducedMotion(false);
    setRootReduceMotion(true);
    const attributeReduced = await mountPlainStreaming(PLAIN_FULL);
    const attributeBody = plainBody(attributeReduced.wrapper);
    const attributeTailCount = attributeBody?.querySelectorAll(TAIL_SELECTOR).length ?? 0;
    const attributeText = attributeBody?.textContent ?? "";
    attributeReduced.wrapper.unmount();
    setRootReduceMotion(false);

    // Assert ③
    expect(attributeTailCount, "V13-3：祖先 data-pds-reduce-motion=\"true\" 下不得渲染尾巴").toBe(0);
    expect(attributeText, "V13-3：属性降级下可见文本必须与开启态逐字一致").toBe(onText);
  });

  it("V13-4: stable + tail 的可见文本必须**逐字等于**已释放文本（全等，不用 includes）", async () => {
    // Arrange：释放速率放大 ⇒ 已释放文本 == 完整 content（确定的真值，不依赖内部 tick 计数）
    const { wrapper } = await mountPlainStreaming(PLAIN_FULL);
    const body = plainBody(wrapper);

    // Assert 自检：先证明尾巴确实存在（否则"拼接 == 全文"可能只是因为全在 stable 里）
    const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);
    expect(tails.length, "V13-4 自检：必须存在尾巴节点，否则本用例对切分实现没有判别力").toBe(1);

    // Assert：**逐字全等** —— 吞字 / 重字 / 顺序错乱都会在这里失败
    expect(
      body?.textContent,
      "V13-4：拼接 stable + tail 的可见文本必须逐字等于已释放文本（纯文本路径最容易吞字/重字）"
    ).toBe(PLAIN_FULL);

    wrapper.unmount();
  });

  it("V13-4b: 慢速释放下 —— 可见文本必须始终是完整文本的**前缀**，且尾巴文本是其后缀", async () => {
    // Arrange：释放速率放慢，制造"只释放了一部分"的帧
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "2");
    const { wrapper } = await mountPlainStreaming(PLAIN_FULL);
    const body = plainBody(wrapper);
    const visible = body?.textContent ?? "";
    const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);

    // Assert 自检：慢速释放必须真的没追平（否则本用例退化成"全文比对"）
    expect(
      visible.length,
      `V13-4b 夹具自检：慢速释放必须尚未追平全文（已释放 ${visible.length} / ${PLAIN_FULL.length}）`
    ).toBeLessThan(PLAIN_FULL.length);
    expect(visible.length, "V13-4b 夹具自检：已释放文本必须非空").toBeGreaterThan(0);

    // Assert ①：可见文本必须是完整文本的前缀（顺序/重复都会被这条抓到）
    expect(
      visible,
      `V13-4b：慢速释放下可见文本必须是全文前缀；实际=${JSON.stringify(visible)}；期望前缀=${JSON.stringify(
        PLAIN_FULL.slice(0, visible.length)
      )}`
    ).toBe(PLAIN_FULL.slice(0, visible.length));

    // Assert ②：若存在尾巴，尾巴文本必须是「已释放文本」的后缀
    if (tails.length === 1) {
      const tailText = tails[0]!.textContent ?? "";
      expect(
        visible.endsWith(tailText),
        `V13-4b：尾巴文本必须是已释放文本的后缀；尾巴=${JSON.stringify(tailText)}；已释放=${JSON.stringify(visible)}`
      ).toBe(true);
    }

    wrapper.unmount();
  });

  it("V13-5: done 帧无残留尾巴、无文本跳动", async () => {
    // Arrange
    const { wrapper, store } = await mountPlainStreaming(PLAIN_FULL, "pending");
    const streamingText = plainBody(wrapper)?.textContent ?? "";
    const streamingTailCount = plainBody(wrapper)?.querySelectorAll(TAIL_SELECTOR).length ?? 0;

    // Act
    await patchAssistant(store, PLAIN_FULL, "done");
    await settleRender();
    const doneBody = plainBody(wrapper);

    // Assert 自检
    expect(streamingTailCount, "V13-5 自检：流式帧必须有尾巴（否则本用例无判别力）").toBe(1);
    expect(doneBody, "V13-5 自检：done 帧必须存在 .markdown-body").not.toBeNull();

    // Assert ①：done 帧不得残留尾巴
    expect(
      doneBody?.querySelectorAll(TAIL_SELECTOR).length ?? 0,
      "V13-5：done 帧不得残留 [data-stream-tail]"
    ).toBe(0);

    // Assert ②：done 帧可见文本 == 流式帧可见文本（逐字），且都等于完整文本
    expect(
      doneBody?.textContent,
      `V13-5：streaming→done 不得产生文本跳动；流式=${JSON.stringify(streamingText.slice(0, 120))}；`
        + `完成=${JSON.stringify((doneBody?.textContent ?? "").slice(0, 120))}`
    ).toBe(streamingText);
    expect(doneBody?.textContent, "V13-5：done 帧可见文本必须等于完整文本").toBe(PLAIN_FULL);

    wrapper.unmount();
  });
});
