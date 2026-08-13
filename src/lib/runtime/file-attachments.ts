// file-attachments domain：类型白名单注册表（单一真相源）+ 文件读取/校验/去重/内容截断/宿主导入。
//
// 注册表与 openspec/changes/add-chat-file-attachment-entry/specs/chat-file-attachment-entry/spec.md
// 的规范表逐字一致；新增支持类型只需加一行注册项，不改路由逻辑。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

export type AttachmentRoute = "image" | "text" | "document";

export type AttachmentHandlerSpec = {
  route: AttachmentRoute;
  /** 白名单 MIME（精确值或 text/* 通配） */
  mimeTypes: string[];
  /** 白名单扩展名（不含点，小写） */
  extensions: string[];
  /** 单文件上限 */
  maxBytes: number;
  /** 注入消息的内容上限（仅 text 路由生效；0 = 不注入） */
  injectMaxBytes: number;
};

/** 与后端 `MAX_TURN_IMAGES = 3` 对齐（runtime/mod.rs:244）。 */
export const MAX_TURN_IMAGES = 3;

/** 文本注入内容预算（spec.md 规范值）。 */
export const TEXT_INJECT_MAX_BYTES = 64 * 1024;

/** 导入目录相对 workspace root 的固定布局（与 PA-080 受控 tmp 逐字一致）。 */
export const IMPORT_RELATIVE_DIR = ".tmp/imports";

/** 图片魔数嗅探判定（与后端 image_artifact.rs 一致，防 MIME 伪造）。 */
export type SniffedImageKind = "png" | "jpeg" | "webp" | "gif";

export const ATTACHMENT_TYPE_REGISTRY: AttachmentHandlerSpec[] = [
  // 图片 → 多模态输入（dataUrl），后端上限 3 张
  {
    route: "image",
    mimeTypes: ["image/png", "image/jpeg", "image/webp", "image/gif"],
    extensions: ["png", "jpg", "jpeg", "webp", "gif"],
    maxBytes: 8 * 1024 * 1024,
    injectMaxBytes: 0
  },
  // 文本可提取格式 → 内容注入（截断标注）
  {
    route: "text",
    mimeTypes: ["text/*", "application/json"],
    extensions: ["md", "txt", "json", "ts", "tsx", "js", "py", "rs", "vue", "css", "html", "csv"],
    maxBytes: 1 * 1024 * 1024,
    injectMaxBytes: TEXT_INJECT_MAX_BYTES
  },
  // 二进制文档 → 引用附着（path + MIME），模型经 workspace_read_document 处理
  {
    route: "document",
    mimeTypes: ["application/pdf", "application/vnd.openxmlformats-officedocument.*"],
    extensions: ["pdf", "docx", "pptx", "xlsx"],
    maxBytes: 20 * 1024 * 1024,
    injectMaxBytes: 0
  }
];

export type PendingAttachmentStatus = "ok" | "error";

export type PendingAttachment = {
  id: string;
  name: string;
  sizeBytes: number;
  /** File.type（或扩展名派生）；图片为嗅探得到的真实类型 */
  mimeType: string;
  route: AttachmentRoute;
  spec: AttachmentHandlerSpec;
  /** 导入后 canonical 路径（Tauri 模式）；浏览器模式为 null */
  path: string | null;
  /** 相对 workspace root 的路径（透传 buildTurnHistory 需要非空 relativePath） */
  relativePath: string | null;
  /** 图片 dataUrl（多模态输入） */
  dataUrl: string | null;
  /** 文本注入内容（截断后） */
  content: string | null;
  /** 内容是否被截断 */
  truncated: boolean;
  /** 文件 lastModified（去重辅助，降低同名同尺寸误伤） */
  lastModified: number | null;
  status: PendingAttachmentStatus;
  errorDetail: string | null;
};

export type ImportResult = {
  /** 导入后的 canonical 路径；浏览器模式 / 复制失败为 null */
  path: string | null;
  /** 相对 workspace root 的引用路径（`.tmp/imports/<name>`）；fallback 目录或浏览器模式为 null */
  relativePath: string | null;
};

/** 由 extension 派生 MIME（File.type 不可用时的兜底）。 */
export function mimeFromExtension(name: string): string {
  const ext = normalizeExtension(name);
  const spec = ATTACHMENT_TYPE_REGISTRY.find((entry) => entry.extensions.includes(ext));
  if (!spec) {
    return "application/octet-stream";
  }
  if (spec.route === "image") {
    return ext === "jpg" ? "image/jpeg" : `image/${ext}`;
  }
  if (spec.route === "text") {
    return ext === "md" || ext === "txt" ? "text/plain" : ext === "json" ? "application/json" : "text/plain";
  }
  if (ext === "pdf") {
    return "application/pdf";
  }
  if (ext === "docx") {
    return "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
  }
  if (ext === "pptx") {
    return "application/vnd.openxmlformats-officedocument.presentationml.presentation";
  }
  if (ext === "xlsx") {
    return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
  }
  return "application/octet-stream";
}

/** 取扩展名（不含点，小写）；无扩展名/空返回 ""。 */
export function normalizeExtension(name: string): string {
  const trimmed = name.trim();
  const dot = trimmed.lastIndexOf(".");
  if (dot <= 0 || dot === trimmed.length - 1) {
    return "";
  }
  return trimmed.slice(dot + 1).toLowerCase();
}

function mimeMatches(spec: AttachmentHandlerSpec, mime: string): boolean {
  const normalized = mime.trim().toLowerCase();
  if (!normalized) {
    return false;
  }
  return spec.mimeTypes.some((pattern) => {
    // text/* 与 application/vnd.openxmlformats-officedocument.* 两种通配前缀
    if (pattern.endsWith("/*") || pattern.endsWith(".*")) {
      return normalized.startsWith(pattern.slice(0, -1));
    }
    return normalized === pattern;
  });
}

/**
 * 解析附件的路由与注册项：扩展名优先、MIME 兜底；扩展名与 MIME 冲突时扩展名胜出。
 * 均未命中返回 null（前端提示"暂不支持该文件类型"）。
 */
export function resolveAttachmentRoute(
  name: string,
  mime: string
): { route: AttachmentRoute; spec: AttachmentHandlerSpec } | null {
  const ext = normalizeExtension(name);
  const byExtension = ATTACHMENT_TYPE_REGISTRY.find((spec) => spec.extensions.includes(ext));
  if (byExtension) {
    return { route: byExtension.route, spec: byExtension };
  }

  const byMime = ATTACHMENT_TYPE_REGISTRY.find((spec) => mimeMatches(spec, mime));
  if (byMime) {
    return { route: byMime.route, spec: byMime };
  }

  return null;
}

/** 嗅探图片真实类型（读文件头，与后端 image_artifact.rs 一致）。 */
export function sniffImageKind(bytes: Uint8Array): SniffedImageKind | null {
  if (bytes.length < 4) {
    return null;
  }
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) {
    return "png";
  }
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    return "jpeg";
  }
  if (bytes.length >= 12) {
    const text = new TextDecoder("latin1").decode(bytes.subarray(0, 12));
    if (text.startsWith("RIFF") && text.slice(8, 12) === "WEBP") {
      return "webp";
    }
  }
  if (bytes.length >= 4) {
    const prefix = String.fromCharCode(bytes[0], bytes[1], bytes[2], bytes[3]);
    if (prefix === "GIF8") {
      return "gif";
    }
  }
  return null;
}

/** 由嗅探得到的图片种类派生规范 MIME（I-2：dataUrl mime 必须与真实内容一致）。 */
export function mimeForSniffed(kind: SniffedImageKind): string {
  switch (kind) {
    case "png":
      return "image/png";
    case "jpeg":
      return "image/jpeg";
    case "webp":
      return "image/webp";
    case "gif":
      return "image/gif";
  }
}

/** 按 UTF-8 字节数截断文本，返回是否被截断。二分查找截断点，避免逐字符 O(n²)。 */
export function truncateTextByBytes(text: string, maxBytes: number): { text: string; truncated: boolean } {
  if (maxBytes <= 0) {
    return { text, truncated: false };
  }
  const encoder = new TextEncoder();
  if (encoder.encode(text).length <= maxBytes) {
    return { text, truncated: false };
  }
  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (encoder.encode(text.slice(0, mid)).length <= maxBytes) {
      low = mid;
    } else {
      high = mid - 1;
    }
  }
  return { text: text.slice(0, low), truncated: true };
}

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  return btoa(binary);
}

/** subarray 防御：只编码 view 实际覆盖的区间（I-12）。 */
function bytesToBase64(bytes: Uint8Array): string {
  const view = new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return arrayBufferToBase64(view.buffer as ArrayBuffer);
}

/** 读取 File 为 ArrayBuffer。 */
export function readFileAsArrayBuffer(file: File): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as ArrayBuffer);
    reader.onerror = () => reject(reader.error ?? new Error("读取文件失败"));
    reader.readAsArrayBuffer(file);
  });
}

/** 读取 File 为 dataUrl（图片多模态输入）。 */
export function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error("读取文件失败"));
    reader.readAsDataURL(file);
  });
}

/** 由 bytes 直接构造 dataUrl（避免图片双次 FileReader）。 */
export function bytesToDataUrl(bytes: Uint8Array, mimeType: string): string {
  return `data:${mimeType || "application/octet-stream"};base64,${bytesToBase64(bytes)}`;
}

export type ImportAttachmentOptions = {
  /** 目标 workspace（PA-079 注册表落地后宿主按此解析 root；当前 fallback 进程 root） */
  workspaceId?: string | null;
};

/**
 * 宿主导入：Tauri 模式将 bytes 交给宿主写入受控导入目录并返回 canonical 路径；
 * 浏览器模式无宿主可写，返回 path=null（仅内存引用）。
 */
export async function importAttachment(
  name: string,
  bytes: Uint8Array,
  mimeType: string,
  options?: ImportAttachmentOptions
): Promise<ImportResult> {
  if (!isTauriAvailable()) {
    return { path: null, relativePath: null };
  }

  try {
    const result = await safeInvoke<ImportResult>("import_attachment", {
      name,
      bytesB64: bytesToBase64(bytes),
      mimeType,
      workspaceId: options?.workspaceId ?? null
    });
    return {
      path: result?.path ?? null,
      relativePath: result?.relativePath ?? null
    };
  } catch (error) {
    const message = String(error);
    const structured = message.includes("code") ? message : `导入失败：${message}`;
    throw new Error(structured);
  }
}

/** 默认文件选择实现：创建 `<input type="file">` 并读取选中的 File。 */
export async function pickFiles(inputOptions?: { accept?: string; multiple?: boolean }): Promise<File[]> {
  return new Promise((resolve, reject) => {
    const input = document.createElement("input");
    input.type = "file";
    if (inputOptions?.accept) {
      input.accept = inputOptions.accept;
    }
    input.multiple = inputOptions?.multiple ?? true;
    input.onchange = () => {
      resolve(Array.from(input.files ?? []));
    };
    input.onerror = () => reject(new Error("文件选择失败"));
    input.click();
  });
}

/** 附件去重键：name + sizeBytes + lastModified（WebView File 无路径，path 由导入派生；
 * 并入 lastModified 降低同名同尺寸不同文件的误伤）。 */
export function attachmentDedupKey(
  attachment: Pick<PendingAttachment, "name" | "sizeBytes" | "lastModified">
): string {
  return `${attachment.name}:${attachment.sizeBytes}:${attachment.lastModified ?? ""}`;
}

/** 附件相对 workspace root 的引用路径（Tauri 导入后固定位于 .tmp/imports/<name>）。 */
export function buildImportRelativePath(name: string): string {
  return `${IMPORT_RELATIVE_DIR}/${name}`;
}
