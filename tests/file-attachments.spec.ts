import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import {
  attachmentDedupKey,
  buildImportRelativePath,
  bytesToDataUrl,
  IMPORT_RELATIVE_DIR,
  mimeFromExtension,
  normalizeExtension,
  resolveAttachmentRoute,
  sniffImageKind,
  truncateTextByBytes,
  type PendingAttachment
} from "@/lib/runtime/file-attachments";
import {
  buildAttachmentMessageBlocks,
  buildAttachmentMetas,
  buildDisplayedUserMessageWithAttachments,
  buildProviderUserMessageWithAttachments
} from "@/lib/runtime/messages";

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

function pendingAttachment(partial: Partial<PendingAttachment> = {}): PendingAttachment {
  return {
    id: partial.id ?? "att-1",
    name: partial.name ?? "a.txt",
    sizeBytes: partial.sizeBytes ?? 10,
    mimeType: partial.mimeType ?? "text/plain",
    route: partial.route ?? "text",
    spec: partial.spec ?? resolveAttachmentRoute("a.txt", "text/plain")!.spec,
    path: partial.path ?? null,
    relativePath: partial.relativePath ?? null,
    dataUrl: partial.dataUrl ?? null,
    content: partial.content ?? null,
    truncated: partial.truncated ?? false,
    status: partial.status ?? "ok",
    errorDetail: partial.errorDetail ?? null
  };
}

function fileBytes(content: string): Uint8Array {
  return new TextEncoder().encode(content);
}

describe("resolveAttachmentRoute", () => {
  it("routes by extension first and MIME fallback", () => {
    expect(resolveAttachmentRoute("photo.png", "image/png")?.route).toBe("image");
    expect(resolveAttachmentRoute("notes.md", "text/plain")?.route).toBe("text");
    expect(resolveAttachmentRoute("report.pdf", "application/pdf")?.route).toBe("document");
    // 无扩展名但 MIME 命中 → MIME 兜底
    expect(resolveAttachmentRoute("untitled", "image/jpeg")?.route).toBe("image");
  });

  it("extension wins over conflicting MIME", () => {
    // .jpg 但 MIME 报 text/plain → 扩展名胜出，路由到 image
    expect(resolveAttachmentRoute("spoofed.jpg", "text/plain")?.route).toBe("image");
  });

  it("rejects unknown types", () => {
    expect(resolveAttachmentRoute("evil.exe", "application/x-msdownload")).toBeNull();
    expect(resolveAttachmentRoute("archive.zip", "application/zip")).toBeNull();
    expect(resolveAttachmentRoute("", "")).toBeNull();
  });

  it("handles uppercase extensions and no-extension files", () => {
    expect(resolveAttachmentRoute("PHOTO.JPG", "")?.route).toBe("image");
    expect(resolveAttachmentRoute("noext", "")).toBeNull();
  });

  it("matches the office-document `.*` wildcard via MIME for no-extension files", () => {
    // 注册表 document MIME 为 application/vnd.openxmlformats-officedocument.*
    expect(
      resolveAttachmentRoute(
        "noext",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
      )?.route
    ).toBe("document");
  });
});

describe("mimeFromExtension / normalizeExtension", () => {
  it("derives canonical MIME from extension", () => {
    expect(mimeFromExtension("a.jpg")).toBe("image/jpeg");
    expect(mimeFromExtension("a.PNG")).toBe("image/png");
    expect(mimeFromExtension("a.pdf")).toBe("application/pdf");
    expect(mimeFromExtension("a.docx")).toBe(
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    );
    expect(mimeFromExtension("noext")).toBe("application/octet-stream");
  });

  it("normalizes extension to lowercase without dot", () => {
    expect(normalizeExtension("Report.PDF")).toBe("pdf");
    expect(normalizeExtension("noext")).toBe("");
    expect(normalizeExtension(".hidden")).toBe("");
  });
});

describe("sniffImageKind", () => {
  it("detects png / jpeg / gif / webp magic bytes", () => {
    expect(sniffImageKind(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a]))).toBe("png");
    expect(sniffImageKind(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe("jpeg");
    expect(sniffImageKind(new TextEncoder().encode("GIF89a"))).toBe("gif");
    const webp = new TextEncoder().encode("RIFF\x00\x00\x00\x00WEBPVP8");
    expect(sniffImageKind(webp)).toBe("webp");
  });

  it("returns null for non-image bytes", () => {
    expect(sniffImageKind(new TextEncoder().encode("hello"))).toBeNull();
    expect(sniffImageKind(new Uint8Array([]))).toBeNull();
  });
});

describe("truncateTextByBytes", () => {
  it("keeps text under budget and marks truncation", () => {
    expect(truncateTextByBytes("hello", 10)).toEqual({ text: "hello", truncated: false });
    const result = truncateTextByBytes("你好世界", 4);
    expect(result.truncated).toBe(true);
    expect(new TextEncoder().encode(result.text).length).toBeLessThanOrEqual(4);
  });

  it("honors zero budget as no-op", () => {
    expect(truncateTextByBytes("hello", 0)).toEqual({ text: "hello", truncated: false });
  });
});

describe("import path helpers", () => {
  it("builds workspace-relative import path", () => {
    expect(buildImportRelativePath("a.txt")).toBe(`${IMPORT_RELATIVE_DIR}/a.txt`);
    expect(IMPORT_RELATIVE_DIR).toBe(".tmp/imports");
  });

  it("dedup key is name + size + lastModified", () => {
    expect(attachmentDedupKey({ name: "a.txt", sizeBytes: 10, lastModified: 123 })).toBe("a.txt:10:123");
    expect(attachmentDedupKey({ name: "a.txt", sizeBytes: 10, lastModified: null })).toBe("a.txt:10:");
  });

  it("builds dataUrl from bytes", () => {
    expect(bytesToDataUrl(fileBytes("x"), "text/plain")).toBe(
      "data:text/plain;base64,eA=="
    );
  });
});

describe("attachment message builders", () => {
  it("builds provider message: text block → document reference → user text", () => {
    const text = pendingAttachment({
      name: "notes.md",
      content: "# hello",
      truncated: false
    });
    const doc = pendingAttachment({
      name: "report.pdf",
      route: "document",
      path: "/ws/.tmp/imports/report.pdf",
      relativePath: ".tmp/imports/report.pdf",
      mimeType: "application/pdf"
    });
    const blocks = buildAttachmentMessageBlocks([text, doc]);
    const message = buildProviderUserMessageWithAttachments("请分析", [], blocks);
    expect(message).toContain("[附件: notes.md]");
    expect(message).toContain("# hello");
    expect(message).toContain("[附件: report.pdf，路径: /ws/.tmp/imports/report.pdf]");
    expect(message).toContain("请分析");
  });

  it("marks truncated text attachments", () => {
    const text = pendingAttachment({ name: "big.md", content: "abc", truncated: true });
    const blocks = buildAttachmentMessageBlocks([text]);
    expect(buildProviderUserMessageWithAttachments("", [], blocks)).toContain(
      "[附件: big.md（内容过长，已截断）]"
    );
  });

  it("falls back to image-only prompt for empty text with images", () => {
    const blocks = { text: [], documents: [] };
    expect(
      buildProviderUserMessageWithAttachments("", [{ dataUrl: "data:image/png;base64,x", mimeType: "image/png" }], blocks)
    ).toBe("请基于附图回答。");
  });

  it("builds displayed summary with image and file counts", () => {
    const text = pendingAttachment({ name: "notes.md", content: "# hello" });
    const blocks = buildAttachmentMessageBlocks([text]);
    const images = [{ dataUrl: "d", mimeType: "image/png", name: "a.png" }];
    const displayed = buildDisplayedUserMessageWithAttachments("hi", images, blocks);
    expect(displayed).toContain("已附图片 1 张");
    expect(displayed).toContain("已附文件 1 个");
  });

  it("projects attachment metas for text/document only", () => {
    const text = pendingAttachment({
      id: "t1",
      name: "notes.md",
      mimeType: "text/plain",
      relativePath: ".tmp/imports/notes.md",
      sizeBytes: 8
    });
    const metas = buildAttachmentMetas([text], "req-1");
    expect(metas).toHaveLength(1);
    expect(metas[0]).toMatchObject({
      name: "notes.md",
      mimeType: "text/plain",
      relativePath: ".tmp/imports/notes.md",
      sizeBytes: 8
    });
  });
});

describe("runtime store addPendingAttachments", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
  });

  it("captures text content and null path in browser mode", async () => {
    const store = useRuntimeStore();
    const file = new File([fileBytes("# hello")], "notes.md", { type: "text/markdown" });
    const { added, errors } = await store.addPendingAttachments([file]);
    expect(added).toBe(1);
    expect(errors).toEqual([]);
    expect(store.pendingAttachments).toHaveLength(1);
    const att = store.pendingAttachments[0];
    expect(att.route).toBe("text");
    expect(att.content).toBe("# hello");
    expect(att.path).toBeNull();
    expect(att.status).toBe("ok");
  });

  it("rejects unsupported types with explicit message", async () => {
    const store = useRuntimeStore();
    const file = new File([new Uint8Array([1, 2, 3])], "evil.exe", { type: "application/x-msdownload" });
    const { added, errors } = await store.addPendingAttachments([file]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("暂不支持");
    expect(store.pendingAttachments).toHaveLength(0);
  });

  it("rejects duplicate by name + size", async () => {
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "a.txt", { type: "text/plain" });
    await store.addPendingAttachments([file]);
    const { added, errors } = await store.addPendingAttachments([file]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("已添加过");
    expect(store.pendingAttachments).toHaveLength(1);
  });

  it("enforces the image count cap at 3", async () => {
    const store = useRuntimeStore();
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0]);
    for (let index = 0; index < 3; index += 1) {
      const file = new File([png], `pic-${index}.png`, { type: "image/png" });
      const { errors } = await store.addPendingAttachments([file]);
      expect(errors).toEqual([]);
    }
    const fourth = new File([png], "pic-3.png", { type: "image/png" });
    const { added, errors } = await store.addPendingAttachments([fourth]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("最多添加");
    expect(store.pendingAttachments.filter((a) => a.route === "image")).toHaveLength(3);
  });

  it("rejects binary documents in browser mode", async () => {
    const store = useRuntimeStore();
    const pdf = new File([new Uint8Array([1, 2, 3])], "report.pdf", { type: "application/pdf" });
    const { added, errors } = await store.addPendingAttachments([pdf]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("预览模式暂不支持二进制文档");
    expect(store.pendingAttachments).toHaveLength(0);
  });

  it("rejects images whose magic bytes do not match declared type", async () => {
    const store = useRuntimeStore();
    const file = new File([new Uint8Array([1, 2, 3, 4])], "fake.png", { type: "image/png" });
    const { added, errors } = await store.addPendingAttachments([file]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("无效图片");
  });

  it("rejects empty image files", async () => {
    const store = useRuntimeStore();
    const file = new File([], "empty.png", { type: "image/png" });
    const { added, errors } = await store.addPendingAttachments([file]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("无效图片");
  });

  it("uses the sniffed image type for the dataUrl mime (PNG content named .jpg)", async () => {
    const store = useRuntimeStore();
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0]);
    const file = new File([png], "fake.jpg", { type: "image/jpeg" });
    const { added, errors } = await store.addPendingAttachments([file]);
    expect(added).toBe(1);
    expect(errors).toEqual([]);
    const att = store.pendingAttachments[0];
    expect(att.mimeType).toBe("image/png");
    expect(att.dataUrl?.startsWith("data:image/png;base64,")).toBe(true);
  });

  it("rejects oversized files", async () => {
    const store = useRuntimeStore();
    // text maxBytes = 1 MiB
    const big = new File([new Uint8Array(2 * 1024 * 1024)], "big.txt", { type: "text/plain" });
    const { added, errors } = await store.addPendingAttachments([big]);
    expect(added).toBe(0);
    expect(errors[0]).toContain("超过大小上限");
  });

  it("calls the host import in tauri mode and records the workspace path", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue({
      path: "C:\\ws\\.tmp\\imports\\notes.md",
      relativePath: ".tmp/imports/notes.md"
    });
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "notes.md", { type: "text/markdown" });
    const { added } = await store.addPendingAttachments([file]);
    expect(added).toBe(1);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("import_attachment", expect.objectContaining({
      name: "notes.md",
      workspaceId: null
    }));
    const att = store.pendingAttachments[0];
    expect(att.path).toBe("C:\\ws\\.tmp\\imports\\notes.md");
    expect(att.relativePath).toBe(".tmp/imports/notes.md");
  });

  it("adopts the host relativePath only when present (fallback → null)", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    // 宿主 fallback（workspace 不可写）→ 返回 path 但 relativePath=null
    tauriMocks.mockSafeInvoke.mockResolvedValue({
      path: "C:\\Temp\\pony-agent\\.tmp\\imports\\notes.md",
      relativePath: null
    });
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "notes.md", { type: "text/markdown" });
    await store.addPendingAttachments([file]);
    const att = store.pendingAttachments[0];
    expect(att.path).toBe("C:\\Temp\\pony-agent\\.tmp\\imports\\notes.md");
    expect(att.relativePath).toBeNull();
  });

  it("surfaces host rejection of a non-default workspaceId as an error chip", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockRejectedValue(
      new Error("[import_attachment_unsupported_workspace] 多 workspace 导入尚未支持")
    );
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "notes.md", { type: "text/markdown" });
    const { added } = await store.addPendingAttachments([file], { workspaceId: "ws-2" });
    expect(added).toBe(0);
    expect(store.pendingAttachments[0].status).toBe("error");
    expect(store.pendingAttachments[0].errorDetail).toContain("unsupported_workspace");
  });

  it("accepts empty text files without host import (tauri mode consistent with browser)", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue({ path: "x", relativePath: "x" });
    const store = useRuntimeStore();
    const empty = new File([], "empty.txt", { type: "text/plain" });
    const { added, errors } = await store.addPendingAttachments([empty]);
    expect(added).toBe(1);
    expect(errors).toEqual([]);
    const att = store.pendingAttachments[0];
    expect(att.content).toBe("");
    expect(att.path).toBeNull();
    expect(att.relativePath).toBeNull();
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalled();
  });

  it("marks import failures as error chips", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockRejectedValue(new Error("[import_attachment_failed] 磁盘已满"));
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "notes.md", { type: "text/markdown" });
    const { added } = await store.addPendingAttachments([file]);
    expect(added).toBe(0);
    expect(store.pendingAttachments[0].status).toBe("error");
    expect(store.pendingAttachments[0].errorDetail).toContain("磁盘已满");
  });

  it("clears pending on remove and on explicit clear", async () => {
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "a.txt", { type: "text/plain" });
    await store.addPendingAttachments([file]);
    expect(store.pendingAttachments).toHaveLength(1);
    store.removePendingAttachment(store.pendingAttachments[0].id);
    expect(store.pendingAttachments).toHaveLength(0);
    await store.addPendingAttachments([file]);
    store.clearPendingAttachments();
    expect(store.pendingAttachments).toHaveLength(0);
  });
});

describe("runtime store submitTurn with attachments", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
  });

  it("builds an attachment-aware turn and clears pending after send", async () => {
    const store = useRuntimeStore();
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0]);
    const imgFile = new File([png], "pic.png", { type: "image/png" });
    const txtFile = new File([fileBytes("# note")], "note.md", { type: "text/markdown" });
    await store.addPendingAttachments([imgFile, txtFile]);
    expect(store.pendingAttachments).toHaveLength(2);

    store.$patch({
      sessionId: "browser-session",
      draftMessage: "分析一下",
      phase: "idle",
      messages: []
    });

    const started = await store.submitTurn();
    expect(started).toBe(true);
    expect(store.pendingAttachments).toHaveLength(0);

    const userMessage = store.messages[0];
    expect(userMessage).toBeDefined();
    expect(userMessage.content).toContain("分析一下");
    expect(userMessage.content).toContain("[已附图片 1 张");
    expect(userMessage.content).toContain("[已附文件 1 个]");
    const textMeta = (userMessage.attachments ?? []).find((attachment) => attachment.name === "note.md");
    expect(textMeta).toBeDefined();
    expect(textMeta?.relativePath).toBeNull();
  });

  it("sends with only attachments and no text (auto summary)", async () => {
    const store = useRuntimeStore();
    const txtFile = new File([fileBytes("hello")], "note.md", { type: "text/markdown" });
    await store.addPendingAttachments([txtFile]);

    store.$patch({ sessionId: "browser-session", draftMessage: "", phase: "idle", messages: [] });

    const started = await store.submitTurn();
    expect(started).toBe(true);
    expect(store.pendingAttachments).toHaveLength(0);
    expect(store.messages[0]?.content).toContain("[已附文件 1 个]");
  });

  it("clears pending attachments on session switch and create", async () => {
    const store = useRuntimeStore();
    const file = new File([fileBytes("hello")], "a.txt", { type: "text/plain" });
    await store.addPendingAttachments([file]);
    expect(store.pendingAttachments).toHaveLength(1);

    await store.switchSession("other-session");
    expect(store.pendingAttachments).toHaveLength(0);

    await store.addPendingAttachments([file]);
    expect(store.pendingAttachments).toHaveLength(1);
    await store.createSession();
    expect(store.pendingAttachments).toHaveLength(0);
  });
});
