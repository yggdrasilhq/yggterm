// Vendored from T3 Code — apps/web/src/components/chat/ExpandedImagePreview.tsx
// Copyright (c) 2026 T3 Tools Inc. — MIT (see ../../LICENSE.t3code).
// Upstream: https://github.com/pingdotgg/t3code (pin: UPSTREAM_COMMIT).
// Local edits are catalogued in ../../NOTICE.md; keep them minimal so a
// re-sync stays a copy rather than a merge.
export interface ExpandedImageItem {
  src: string;
  name: string;
}

export interface ExpandedImagePreview {
  images: ExpandedImageItem[];
  index: number;
}

export function buildExpandedImagePreview(
  images: ReadonlyArray<{ id: string; name: string; previewUrl?: string }>,
  selectedImageId: string,
): ExpandedImagePreview | null {
  const previewableImages = images.flatMap((image) =>
    image.previewUrl ? [{ id: image.id, src: image.previewUrl, name: image.name }] : [],
  );
  if (previewableImages.length === 0) {
    return null;
  }
  const selectedIndex = previewableImages.findIndex((image) => image.id === selectedImageId);
  if (selectedIndex < 0) {
    return null;
  }
  return {
    images: previewableImages.map((image) => ({
      src: image.src,
      name: image.name,
    })),
    index: selectedIndex,
  };
}
