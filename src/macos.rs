//! macOS implementation: `NSDraggingSession` with `NSURL` writers for existing
//! files and `NSFilePromiseProvider` writers for everything produced on drop.

#![allow(deprecated)]

use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use cocoa::base::{id, nil, BOOL, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Protocol, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde_json::json;

use crate::{
    emit, DragItem, DragOptions, DragSession, Error, PromiseWriter, EVENT_DROPPED, EVENT_ENDED,
    EVENT_ERROR,
};

const NS_DRAG_OPERATION_NONE: u64 = 0;
const NS_DRAG_OPERATION_COPY: u64 = 1;
const NS_DRAGGING_CONTEXT_WITHIN_APP: i64 = 1;
const NS_EVENT_LEFT_MOUSE_DRAGGED: u64 = 6;
const NS_QOS_USER_INITIATED: i64 = 0x19;

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn UTTypeCreatePreferredIdentifierForTag(tag_class: id, tag: id, conforming_to: id) -> id;
}

enum Source {
    Copy(PathBuf),
    Data(Vec<u8>),
    Write(PromiseWriter),
}

struct Pending {
    session: u64,
    name: String,
    source: Source,
}

struct SessionState {
    delegate: usize,
    pending: usize,
    ended: bool,
}

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

fn lock<T>(m: &'static OnceLock<Mutex<T>>) -> MutexGuard<'static, T>
where
    T: Default,
{
    m.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

static PROMISES: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
static SESSIONS: OnceLock<Mutex<HashMap<u64, SessionState>>> = OnceLock::new();
/// A view whose drop registration is suspended, and the dragged types it had.
type SuspendedView = (usize, usize);
static SUSPENDED: OnceLock<Mutex<HashMap<u64, Vec<SuspendedView>>>> = OnceLock::new();

pub(crate) fn is_main_thread() -> bool {
    unsafe {
        let main: BOOL = msg_send![class!(NSThread), isMainThread];
        main != NO
    }
}

/// Starts a drag session. Must run on the main thread.
pub(crate) fn start(
    ns_window: usize,
    items: Vec<DragItem>,
    options: DragOptions,
) -> Result<DragSession, Error> {
    let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        start_inner(session, ns_window as id, items, options)
    }))
    .unwrap_or_else(|p| Err(Error::Platform(panic_message(p))));
    match result {
        Ok(()) => Ok(DragSession { id: session }),
        Err(e) => {
            forget_session(session);
            Err(e)
        }
    }
}

unsafe fn start_inner(
    session: u64,
    window: id,
    items: Vec<DragItem>,
    options: DragOptions,
) -> Result<(), Error> {
    if window == nil {
        return Err(Error::Platform("the window has no NSWindow".into()));
    }
    let pressed: u64 = msg_send![class!(NSEvent), pressedMouseButtons];
    if pressed == 0 {
        return Err(Error::NoMouse);
    }
    let view: id = msg_send![window, contentView];
    if view == nil {
        return Err(Error::Platform("the window has no content view".into()));
    }

    let pool: id = msg_send![class!(NSAutoreleasePool), new];
    let result = begin(session, window, view, items, &options);
    let _: () = msg_send![pool, drain];
    result
}

unsafe fn begin(
    session: u64,
    window: id,
    view: id,
    items: Vec<DragItem>,
    options: &DragOptions,
) -> Result<(), Error> {
    let event = drag_event(window);
    let in_window: NSPoint = msg_send![event, locationInWindow];
    let origin: NSPoint = msg_send![view, convertPoint: in_window fromView: nil];

    // One delegate per session: dragging source + promise provider delegate.
    let delegate: id = msg_send![delegate_class(), new];
    (*delegate).set_ivar::<u64>("sessionId", session);
    (*delegate).set_ivar::<u8>("allowSelfDrop", options.allow_self_drop as u8);
    let promise_count = items.iter().filter(|i| !matches!(i, DragItem::File { .. })).count();
    lock(&SESSIONS).insert(
        session,
        SessionState { delegate: delegate as usize, pending: promise_count, ended: false },
    );
    if !options.allow_self_drop {
        suspend_drop_targets(session, view);
    }

    let size = options.icon_size.unwrap_or(64.0).clamp(16.0, 512.0);
    let custom_icon = custom_icon(options);
    let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
    let dragging_items: id = msg_send![class!(NSMutableArray), array];

    for (index, item) in items.into_iter().enumerate() {
        let name = item.display_name();
        let (writer, icon): (id, id) = match item {
            DragItem::File { path } => {
                let p = nsstring(&path.to_string_lossy());
                let url: id = msg_send![class!(NSURL), fileURLWithPath: p];
                let icon: id = msg_send![workspace, iconForFile: p];
                (url, icon)
            }
            other => {
                let (source, icon) = match other {
                    DragItem::Copy { source, .. } => {
                        let p = nsstring(&source.to_string_lossy());
                        let icon: id = msg_send![workspace, iconForFile: p];
                        (Source::Copy(source), icon)
                    }
                    DragItem::Data { data, .. } => (Source::Data(data), type_icon(workspace, &name)),
                    DragItem::Promise { write, .. } => {
                        (Source::Write(write), type_icon(workspace, &name))
                    }
                    DragItem::File { .. } => unreachable!(),
                };
                let key = format!("{session}:{index}");
                let file_type = uti_for_name(&name);
                lock(&PROMISES).insert(key.clone(), Pending { session, name, source });
                let provider: id = msg_send![class!(NSFilePromiseProvider), alloc];
                let provider: id =
                    msg_send![provider, initWithFileType: file_type delegate: delegate];
                let _: () = msg_send![provider, setUserInfo: nsstring(&key)];
                let provider: id = msg_send![provider, autorelease];
                (provider, icon)
            }
        };

        let icon = if custom_icon != nil { custom_icon } else { icon };
        let icon: id = if icon != nil { msg_send![icon, copy] } else { nil };
        if icon != nil {
            let _: () = msg_send![icon, setSize: NSSize::new(size, size)];
            let _: id = msg_send![icon, autorelease];
        }
        let shift = (index.min(8) as f64) * 6.0;
        let frame = NSRect::new(
            NSPoint::new(origin.x - size / 2.0 + shift, origin.y - size / 2.0 - shift),
            NSSize::new(size, size),
        );
        let dragging_item: id = msg_send![class!(NSDraggingItem), alloc];
        let dragging_item: id = msg_send![dragging_item, initWithPasteboardWriter: writer];
        let _: () = msg_send![dragging_item, setDraggingFrame: frame contents: icon];
        let _: () = msg_send![dragging_items, addObject: dragging_item];
        let _: () = msg_send![dragging_item, release];
    }

    let drag: id = msg_send![view, beginDraggingSessionWithItems: dragging_items
                                                          event: event
                                                         source: delegate];
    if drag == nil {
        return Err(Error::Platform("beginDraggingSession returned nil".into()));
    }
    let _: () = msg_send![drag, setAnimatesToStartingPositionsOnCancelOrFail: YES];
    Ok(())
}

/// AppKit needs a mouse event of this window. When the IPC call is handled the
/// current event is usually the mouse-down/drag that triggered it; otherwise
/// synthesize a drag event at the current mouse location.
unsafe fn drag_event(window: id) -> id {
    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let current: id = msg_send![app, currentEvent];
    if current != nil {
        let kind: u64 = msg_send![current, type];
        let event_window: id = msg_send![current, window];
        // left/right/other mouse down and dragged
        if matches!(kind, 1 | 3 | 6 | 7 | 25 | 27) && event_window == window {
            return current;
        }
    }
    let location: NSPoint = msg_send![window, mouseLocationOutsideOfEventStream];
    let number: i64 = msg_send![window, windowNumber];
    let info: id = msg_send![class!(NSProcessInfo), processInfo];
    let uptime: f64 = msg_send![info, systemUptime];
    msg_send![class!(NSEvent), mouseEventWithType: NS_EVENT_LEFT_MOUSE_DRAGGED
                                          location: location
                                     modifierFlags: 0u64
                                         timestamp: uptime
                                      windowNumber: number
                                           context: nil
                                       eventNumber: 0i64
                                        clickCount: 1i64
                                          pressure: 1.0f32]
}

unsafe fn custom_icon(options: &DragOptions) -> id {
    let image: id = msg_send![class!(NSImage), alloc];
    let image: id = if let Some(bytes) = &options.icon_data {
        let data: id = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() as *const c_void
                                                         length: bytes.len()];
        msg_send![image, initWithData: data]
    } else if let Some(path) = &options.icon {
        msg_send![image, initWithContentsOfFile: nsstring(&path.to_string_lossy())]
    } else {
        let _: () = msg_send![image, release];
        return nil;
    };
    if image == nil {
        return nil;
    }
    msg_send![image, autorelease]
}

unsafe fn type_icon(workspace: id, name: &str) -> id {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().into_owned());
    let kind = nsstring(ext.as_deref().unwrap_or("public.data"));
    msg_send![workspace, iconForFileType: kind]
}

unsafe fn uti_for_name(name: &str) -> id {
    if let Some(ext) = Path::new(name).extension() {
        let uti = UTTypeCreatePreferredIdentifierForTag(
            nsstring("public.filename-extension"),
            nsstring(&ext.to_string_lossy()),
            nil,
        );
        if uti != nil {
            return msg_send![uti, autorelease];
        }
    }
    nsstring("public.data")
}

unsafe fn nsstring(s: &str) -> id {
    let s: id = NSString::alloc(nil).init_str(s);
    msg_send![s, autorelease]
}

unsafe fn rust_string(s: id) -> Option<String> {
    if s == nil {
        return None;
    }
    let c: *const c_char = msg_send![s, UTF8String];
    if c.is_null() {
        None
    } else {
        Some(CStr::from_ptr(c).to_string_lossy().into_owned())
    }
}

fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "native drag panicked".into())
}

// ---- session bookkeeping -------------------------------------------------

fn release_delegate(ptr: usize) {
    // autorelease: the delegate may be the object whose method is running.
    unsafe {
        let _: id = msg_send![(ptr as id), autorelease];
    }
}

fn drop_promises_of(session: u64) {
    lock(&PROMISES).retain(|_, p| p.session != session);
}

/// WKWebView accepts a drop whatever operation mask the source reports, so the
/// mask alone cannot keep a drag out of its own window. Instead, the window's
/// views stop accepting drops until the drag ends. Must run on the main thread.
unsafe fn suspend_drop_targets(session: u64, root: id) {
    let mut saved = Vec::new();
    let mut stack = vec![root];
    while let Some(view) = stack.pop() {
        let types: id = msg_send![view, registeredDraggedTypes];
        let count: usize = if types == nil { 0 } else { msg_send![types, count] };
        if count > 0 {
            let types: id = msg_send![types, copy];
            let _: id = msg_send![view, retain];
            let _: () = msg_send![view, unregisterDraggedTypes];
            saved.push((view as usize, types as usize));
        }
        let subviews: id = msg_send![view, subviews];
        let children: usize = if subviews == nil { 0 } else { msg_send![subviews, count] };
        for i in 0..children {
            let child: id = msg_send![subviews, objectAtIndex: i];
            stack.push(child);
        }
    }
    if !saved.is_empty() {
        lock(&SUSPENDED).insert(session, saved);
    }
}

/// Give the views back the drop registration taken by `suspend_drop_targets`.
/// Must run on the main thread.
fn restore_drop_targets(session: u64) {
    let Some(saved) = lock(&SUSPENDED).remove(&session) else { return };
    for (view, types) in saved {
        unsafe {
            let (view, types) = (view as id, types as id);
            let _: () = msg_send![view, registerForDraggedTypes: types];
            let _: () = msg_send![types, release];
            let _: () = msg_send![view, release];
        }
    }
}

/// Drop everything of a session that failed to start. Runs on the main thread.
fn forget_session(session: u64) {
    restore_drop_targets(session);
    drop_promises_of(session);
    if let Some(state) = lock(&SESSIONS).remove(&session) {
        release_delegate(state.delegate);
    }
}

fn end_session(session: u64, cancelled: bool) {
    let mut sessions = lock(&SESSIONS);
    let Some(state) = sessions.get_mut(&session) else { return };
    state.ended = true;
    if cancelled || state.pending == 0 {
        let state = sessions.remove(&session).expect("session present");
        drop(sessions);
        if cancelled {
            drop_promises_of(session);
        }
        release_delegate(state.delegate);
    }
}

fn promise_done(session: u64) {
    let mut sessions = lock(&SESSIONS);
    let Some(state) = sessions.get_mut(&session) else { return };
    state.pending = state.pending.saturating_sub(1);
    if state.ended && state.pending == 0 {
        let state = sessions.remove(&session).expect("session present");
        drop(sessions);
        release_delegate(state.delegate);
    }
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn fulfil(key: &str, dest: &Path) -> Result<(), String> {
    let pending = lock(&PROMISES)
        .remove(key)
        .ok_or_else(|| "this drag item is no longer available".to_string())?;
    let result = catch_unwind(AssertUnwindSafe(|| match &pending.source {
        Source::Copy(src) => copy_recursive(src, dest).map_err(|e| e.to_string()),
        Source::Data(bytes) => std::fs::write(dest, bytes).map_err(|e| e.to_string()),
        Source::Write(write) => write(dest),
    }))
    .unwrap_or_else(|p| Err(panic_message(p)));
    let path = dest.to_string_lossy();
    match &result {
        Ok(()) => emit(
            EVENT_DROPPED,
            json!({ "session": pending.session, "name": pending.name, "path": path }),
        ),
        Err(message) => emit(
            EVENT_ERROR,
            json!({ "session": pending.session, "name": pending.name, "path": path, "message": message }),
        ),
    }
    promise_done(pending.session);
    result
}

// ---- Objective-C delegate --------------------------------------------------

#[repr(C)]
struct BlockLiteral {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*mut BlockLiteral, id),
}

/// Calls the `void (^)(NSError *)` completion handler.
unsafe fn complete(block: id, error: Option<&str>) {
    if block == nil {
        return;
    }
    let ns_error: id = match error {
        None => nil,
        Some(message) => {
            let info: id = msg_send![class!(NSDictionary),
                dictionaryWithObject: nsstring(message)
                              forKey: nsstring("NSLocalizedDescription")];
            msg_send![class!(NSError), errorWithDomain: nsstring("tauri-plugin-dragout")
                                                  code: 1i64
                                              userInfo: info]
        }
    };
    let literal = block as *mut BlockLiteral;
    ((*literal).invoke)(literal, ns_error);
}

extern "C" fn file_name_for_type(_this: &Object, _sel: Sel, provider: id, _file_type: id) -> id {
    let name = catch_unwind(AssertUnwindSafe(|| unsafe {
        let key: id = msg_send![provider, userInfo];
        rust_string(key).and_then(|k| lock(&PROMISES).get(&k).map(|p| p.name.clone()))
    }))
    .ok()
    .flatten()
    .unwrap_or_else(|| "Untitled".into());
    unsafe { nsstring(&name) }
}

extern "C" fn write_promise(_this: &Object, _sel: Sel, provider: id, url: id, completion: id) {
    let outcome = catch_unwind(AssertUnwindSafe(|| unsafe {
        let key: id = msg_send![provider, userInfo];
        let key = rust_string(key).ok_or("missing promise key")?;
        let dest: id = msg_send![url, path];
        let dest = rust_string(dest).ok_or("missing destination")?;
        fulfil(&key, Path::new(&dest))
    }))
    .unwrap_or_else(|p| Err(panic_message(p)));
    unsafe { complete(completion, outcome.err().as_deref()) };
}

extern "C" fn operation_queue(_this: &Object, _sel: Sel, _provider: id) -> id {
    static QUEUE: OnceLock<usize> = OnceLock::new();
    *QUEUE.get_or_init(|| unsafe {
        let queue: id = msg_send![class!(NSOperationQueue), new];
        let _: () = msg_send![queue, setQualityOfService: NS_QOS_USER_INITIATED];
        let _: () = msg_send![queue, setName: nsstring("tauri-plugin-dragout")];
        queue as usize
    }) as id
}

extern "C" fn source_operation_mask(this: &Object, _sel: Sel, _session: id, context: i64) -> u64 {
    if context == NS_DRAGGING_CONTEXT_WITHIN_APP {
        let allow: u8 = unsafe { *this.get_ivar("allowSelfDrop") };
        if allow == 0 {
            return NS_DRAG_OPERATION_NONE;
        }
    }
    NS_DRAG_OPERATION_COPY
}

extern "C" fn ended_at_point(this: &Object, _sel: Sel, _session: id, _point: NSPoint, op: u64) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let session: u64 = unsafe { *this.get_ivar("sessionId") };
        let operation = match op {
            0 => "none",
            o if o & 1 != 0 => "copy",
            o if o & 16 != 0 => "move",
            o if o & 2 != 0 => "link",
            o if o & 32 != 0 => "delete",
            _ => "other",
        };
        restore_drop_targets(session);
        emit(
            EVENT_ENDED,
            json!({ "session": session, "operation": operation, "cancelled": op == 0 }),
        );
        end_session(session, op == 0);
    }));
}

fn delegate_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        const NAME: &str = "TauriPluginDragoutDelegate";
        if let Some(existing) = Class::get(NAME) {
            return existing;
        }
        let mut decl = ClassDecl::new(NAME, class!(NSObject)).expect("declare delegate class");
        for protocol in ["NSDraggingSource", "NSFilePromiseProviderDelegate"] {
            if let Some(p) = Protocol::get(protocol) {
                decl.add_protocol(p);
            }
        }
        decl.add_ivar::<u64>("sessionId");
        decl.add_ivar::<u8>("allowSelfDrop");
        decl.add_method(
            sel!(filePromiseProvider:fileNameForType:),
            file_name_for_type as extern "C" fn(&Object, Sel, id, id) -> id,
        );
        decl.add_method(
            sel!(filePromiseProvider:writePromiseToURL:completionHandler:),
            write_promise as extern "C" fn(&Object, Sel, id, id, id),
        );
        decl.add_method(
            sel!(operationQueueForFilePromiseProvider:),
            operation_queue as extern "C" fn(&Object, Sel, id) -> id,
        );
        decl.add_method(
            sel!(draggingSession:sourceOperationMaskForDraggingContext:),
            source_operation_mask as extern "C" fn(&Object, Sel, id, i64) -> u64,
        );
        decl.add_method(
            sel!(draggingSession:endedAtPoint:operation:),
            ended_at_point as extern "C" fn(&Object, Sel, id, NSPoint, u64),
        );
        decl.register()
    })
}
