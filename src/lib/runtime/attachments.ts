// 附件 domain：附件资产克隆与过滤。
import type { AttachmentAsset, AttachmentAssetFilter } from "../../types/runtime";

export function cloneAttachmentAssets(assets?: AttachmentAsset[] | null) {
  return (assets ?? []).map((asset) => ({ ...asset }));
}

export function filterAttachmentAssets(assets: AttachmentAsset[], filter?: AttachmentAssetFilter | null) {
  const normalizedMime = filter?.mimeType?.trim().toLowerCase() ?? "";
  const normalizedName = filter?.nameContains?.trim().toLowerCase() ?? "";
  const requestedStatuses = new Set(filter?.statuses ?? []);

  const filtered = assets.filter((asset) => {
    if (filter?.sessionId?.trim() && asset.sessionId !== filter.sessionId.trim()) {
      return false;
    }

    if (normalizedMime && !asset.mimeType.toLowerCase().includes(normalizedMime)) {
      return false;
    }

    if (normalizedName) {
      const assetName = asset.name?.toLowerCase() ?? "";
      const relativePath = asset.relativePath.toLowerCase();
      if (!assetName.includes(normalizedName) && !relativePath.includes(normalizedName)) {
        return false;
      }
    }

    if (filter?.createdAfterMs != null && asset.createdAtMs < filter.createdAfterMs) {
      return false;
    }

    if (filter?.createdBeforeMs != null && asset.createdAtMs > filter.createdBeforeMs) {
      return false;
    }

    if (requestedStatuses.size > 0) {
      const status = asset.status ?? "active";
      if (!requestedStatuses.has(status)) {
        return false;
      }
    }

    return true;
  });

  filtered.sort((left, right) => {
    if (right.createdAtMs !== left.createdAtMs) {
      return right.createdAtMs - left.createdAtMs;
    }
    return left.id.localeCompare(right.id);
  });

  if (filter?.limit != null) {
    return filtered.slice(0, filter.limit);
  }

  return filtered;
}
