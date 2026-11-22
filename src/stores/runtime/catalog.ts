// D8 健康与能力目录（4 键 / 0 getters / 5 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**（唯一改动：import 路径按新层级调整、
// action 签名补 `this: RuntimeStoreThis` 形参），语义零改动。键清单与签名真源见 `./types.ts`。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import { debugLog } from "@/lib/runtime/utils";
import {
  createAvailableTools,
  createCapabilities,
  createCapabilitySources,
  defaultAvailableTools
} from "@/lib/runtime/browser-preview";
import type { AvailableTool, CapabilitySourceView, CapabilityView } from "@/types/runtime";
import type { CatalogActions, CatalogGetters, RuntimeStoreThis } from "./types";

/** D8 健康与能力目录 getters（本域 0 条，见事实图 §2）。 */
export const catalogGetters = {} as CatalogGetters;

/** D8 健康与能力目录 actions（键清单见 `types.ts` 的 `CatalogActions` 与事实图 §3）。 */
export const catalogActions = {
    async fetchAvailableTools(this: RuntimeStoreThis) {
      if (this.availableTools.length > 0 && isTauriAvailable()) {
        const defaultToolNames = new Set(defaultAvailableTools.map((tool) => tool.name));
        const hasDefaultOnly =
          this.availableTools.length === defaultAvailableTools.length &&
          this.availableTools.every((tool) => defaultToolNames.has(tool.name));
        if (!hasDefaultOnly) {
          return;
        }
      }

      try {
        this.availableTools = isTauriAvailable()
          ? await safeInvoke<AvailableTool[]>("list_available_tools")
          : createAvailableTools();
        debugLog("tools:ok", {
          count: this.availableTools.length
        });
      } catch (error) {
        this.availableTools = createAvailableTools();
        debugLog("tools:error", {
          error: String(error)
        });
      }
    },
    async fetchCapabilitySources(this: RuntimeStoreThis) {
      try {
        this.capabilitySources = isTauriAvailable()
          ? await safeInvoke<CapabilitySourceView[]>("list_capability_sources")
          : createCapabilitySources();
        debugLog("capability-sources:ok", {
          count: this.capabilitySources.length
        });
      } catch (error) {
        this.capabilitySources = createCapabilitySources();
        debugLog("capability-sources:error", {
          error: String(error)
        });
      }
    },
    async fetchCapabilities(this: RuntimeStoreThis, filter?: { sourceId?: string | null; kind?: string | null }) {
      const sourceId = filter?.sourceId?.trim() || null;
      const kind = filter?.kind?.trim() || null;

      try {
        this.capabilities = isTauriAvailable()
          ? await safeInvoke<CapabilityView[]>("list_capabilities", {
            sourceId,
            kind
          })
          : createCapabilities().filter((capability) => {
            if (sourceId && capability.sourceId !== sourceId) {
              return false;
            }
            if (kind && capability.kind !== kind) {
              return false;
            }
            return true;
          });
        debugLog("capabilities:ok", {
          count: this.capabilities.length,
          sourceId,
          kind
        });
      } catch (error) {
        this.capabilities = createCapabilities().filter((capability) => {
          if (sourceId && capability.sourceId !== sourceId) {
            return false;
          }
          if (kind && capability.kind !== kind) {
            return false;
          }
          return true;
        });
        debugLog("capabilities:error", {
          sourceId,
          kind,
          error: String(error)
        });
      }
    },
    async inspectCapability(this: RuntimeStoreThis, capabilityId: string) {
      if (!capabilityId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        return createCapabilities().find((capability) => capability.capabilityId === capabilityId) ?? null;
      }

      try {
        return await safeInvoke<CapabilityView | null>("inspect_capability", {
          capabilityId
        });
      } catch (error) {
        debugLog("capability:inspect:error", {
          capabilityId,
          error: String(error)
        });
        return createCapabilities().find((capability) => capability.capabilityId === capabilityId) ?? null;
      }
    },
    async inspectCapabilitySource(this: RuntimeStoreThis, sourceId: string) {
      if (!sourceId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        return createCapabilitySources().find((source) => source.sourceId === sourceId) ?? null;
      }

      try {
        return await safeInvoke<CapabilitySourceView | null>("inspect_capability_source", {
          sourceId
        });
      } catch (error) {
        debugLog("capability-source:inspect:error", {
          sourceId,
          error: String(error)
        });
        return createCapabilitySources().find((source) => source.sourceId === sourceId) ?? null;
      }
    },
} as CatalogActions;
