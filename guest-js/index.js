// JavaScript API for tauri-plugin-dragout. Plain ESM, no build step.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const CHUNK = 0x8000;

function bytesToBase64(bytes) {
  let binary = '';
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

// string (base64 or data: URL) | Uint8Array | ArrayBuffer | Blob | <canvas>
async function toBase64(data) {
  if (data == null) return undefined;
  if (typeof data === 'string') return data;
  if (typeof HTMLCanvasElement !== 'undefined' && data instanceof HTMLCanvasElement) {
    return data.toDataURL('image/png');
  }
  if (typeof Blob !== 'undefined' && data instanceof Blob) {
    return bytesToBase64(new Uint8Array(await data.arrayBuffer()));
  }
  if (data instanceof ArrayBuffer) return bytesToBase64(new Uint8Array(data));
  if (ArrayBuffer.isView(data)) {
    return bytesToBase64(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  }
  throw new TypeError('dragout: unsupported data type');
}

async function normalizeItem(item) {
  if (item.type === 'data') return { ...item, data: await toBase64(item.data) };
  return item;
}

/**
 * Start a native drag. Call it while the mouse button is held
 * (from a pointerdown/pointermove handler), or use `draggable()`.
 */
export async function startDrag(items, options = {}) {
  const list = Array.isArray(items) ? items : [items];
  const prepared = await Promise.all(list.map(normalizeItem));
  const opts = { ...options };
  if (opts.iconData != null) opts.iconData = await toBase64(opts.iconData);
  return invoke('plugin:dragout|start_drag', { items: prepared, options: opts });
}

/** Whether native drag-out works on this platform (macOS today). */
export function isSupported() {
  return invoke('plugin:dragout|is_supported');
}

/** A promised item was written: `{ session, name, path }`. */
export function onDropped(handler) {
  return listen('dragout://dropped', (e) => handler(e.payload));
}

/** A drag session ended: `{ session, operation, cancelled }`. */
export function onEnded(handler) {
  return listen('dragout://ended', (e) => handler(e.payload));
}

/** Writing a promised item failed: `{ session, name, path, message }`. */
export function onError(handler) {
  return listen('dragout://error', (e) => handler(e.payload));
}

/**
 * Make an element drag out natively. Disables the webview's own HTML drag
 * for it and starts the native drag once the pointer moves `threshold` px.
 *
 * @param element  element to drag from
 * @param items    items, or a function (event) => items | Promise<items>
 * @param options  DragOptions plus { threshold, onStart, onError }
 * @returns        function that removes the listeners
 */
export function draggable(element, items, options = {}) {
  const { threshold = 4, onStart, onError: onFail, ...dragOptions } = options;
  let origin = null;

  const stop = () => {
    origin = null;
    window.removeEventListener('pointermove', onMove, true);
    window.removeEventListener('pointerup', stop, true);
    window.removeEventListener('pointercancel', stop, true);
  };

  async function onMove(event) {
    if (!origin) return;
    if ((event.buttons & 1) === 0) return stop();
    const dx = event.clientX - origin.x;
    const dy = event.clientY - origin.y;
    if (dx * dx + dy * dy < threshold * threshold) return;
    stop();
    try {
      const list = typeof items === 'function' ? await items(event) : items;
      if (!list || (Array.isArray(list) && list.length === 0)) return;
      const session = await startDrag(list, dragOptions);
      if (onStart) onStart(session);
    } catch (err) {
      if (onFail) onFail(err);
      else console.error('[dragout]', err);
    }
  }

  const onDown = (event) => {
    if (event.button !== 0) return;
    // Keep WebKit from starting a text selection before the drag threshold is reached.
    event.preventDefault();
    origin = { x: event.clientX, y: event.clientY };
    window.addEventListener('pointermove', onMove, true);
    window.addEventListener('pointerup', stop, true);
    window.addEventListener('pointercancel', stop, true);
  };
  const onDragStart = (event) => event.preventDefault();

  element.setAttribute('draggable', 'false');
  element.addEventListener('pointerdown', onDown);
  element.addEventListener('dragstart', onDragStart);

  return () => {
    stop();
    element.removeEventListener('pointerdown', onDown);
    element.removeEventListener('dragstart', onDragStart);
  };
}
