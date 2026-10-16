import type { UnlistenFn } from '@tauri-apps/api/event';

/** Base64 string or data: URL, raw bytes, a Blob, or (for icons) a canvas. */
export type BinaryLike = string | Uint8Array | ArrayBuffer | ArrayBufferView | Blob;

export type DragItem =
  /** An existing file or folder; the target receives the original. */
  | { type: 'file'; path: string }
  /** Copied to the drop location on drop, optionally renamed. */
  | { type: 'copy'; source: string; fileName?: string }
  /** These bytes are written to `fileName` at the drop location. */
  | { type: 'data'; fileName: string; data: BinaryLike };

export interface DragOptions {
  /** Path to an image used as the drag preview. */
  icon?: string;
  /** Image used as the drag preview (e.g. a canvas snapshot of the element). */
  iconData?: BinaryLike | HTMLCanvasElement;
  /** Preview size in points, default 64. */
  iconSize?: number;
  /** Allow dropping onto this app's own windows, default false. */
  allowSelfDrop?: boolean;
}

export interface DragSession {
  id: number;
}

export interface DroppedEvent {
  session: number;
  name: string;
  path: string;
}

export interface EndedEvent {
  session: number;
  operation: 'none' | 'copy' | 'move' | 'link' | 'delete' | 'other';
  cancelled: boolean;
}

export interface ErrorEvent extends DroppedEvent {
  message: string;
}

export interface DraggableOptions extends DragOptions {
  /** Pointer movement in px before the drag starts, default 4. */
  threshold?: number;
  onStart?: (session: DragSession) => void;
  onError?: (error: unknown) => void;
}

export function startDrag(items: DragItem | DragItem[], options?: DragOptions): Promise<DragSession>;
export function isSupported(): Promise<boolean>;
export function onDropped(handler: (event: DroppedEvent) => void): Promise<UnlistenFn>;
export function onEnded(handler: (event: EndedEvent) => void): Promise<UnlistenFn>;
export function onError(handler: (event: ErrorEvent) => void): Promise<UnlistenFn>;
export function draggable(
  element: HTMLElement,
  items: DragItem | DragItem[] | ((event: PointerEvent) => DragItem | DragItem[] | Promise<DragItem | DragItem[]>),
  options?: DraggableOptions,
): () => void;
