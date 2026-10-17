//! Native macOS Quick Look for cached mail attachments.
//!
//! QuickLookUI is linked directly so this stays a small platform adapter and
//! does not introduce a second PDF/document renderer. `QLPreviewPanel` keeps
//! its data source unretained, therefore the current source is retained in a
//! process-global pointer and replaced only on the main thread.

use std::sync::atomic::{AtomicPtr, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL};

#[link(name = "QuickLookUI", kind = "framework")]
unsafe extern "C" {}

struct SourceIvars {
    url: Retained<NSURL>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. All access happens on
    // the main thread and the retained URL lives in the declared ivars.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatchelQuickLookSource"]
    #[ivars = SourceIvars]
    struct Source;

    impl Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_items(&self, _panel: &AnyObject) -> isize {
            1
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_item(&self, _panel: &AnyObject, _index: isize) -> Retained<NSURL> {
            self.ivars().url.clone()
        }
    }

    unsafe impl NSObjectProtocol for Source {}
);

impl Source {
    fn new(path: &str, mtm: MainThreadMarker) -> Retained<Self> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let this = mtm.alloc::<Self>().set_ivars(SourceIvars { url });
        // SAFETY: plain NSObject initialization for the ivars above.
        unsafe { msg_send![super(this), init] }
    }
}

static CURRENT_SOURCE: AtomicPtr<Source> = AtomicPtr::new(std::ptr::null_mut());

pub fn toggle(path: &str) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("Quick Look must run on the main thread".to_string());
    };
    let panel_class = AnyClass::get(c"QLPreviewPanel")
        .ok_or_else(|| "Quick Look is unavailable on this macOS version".to_string())?;

    // SAFETY: QLPreviewPanel is present after linking QuickLookUI and these are
    // stable AppKit/QuickLookUI selectors with the declared signatures.
    unsafe {
        let panel: Retained<AnyObject> = msg_send![panel_class, sharedPreviewPanel];
        let visible: bool = msg_send![&*panel, isVisible];
        if visible {
            let _: () = msg_send![&*panel, orderOut: Option::<&AnyObject>::None];
            return Ok(());
        }

        let source = Source::new(path, mtm);
        let _: () = msg_send![&*panel, setDataSource: &*source];
        let _: () = msg_send![&*panel, reloadData];
        let _: () = msg_send![&*panel, setCurrentPreviewItemIndex: 0isize];
        let _: () = msg_send![&*panel, makeKeyAndOrderFront: Option::<&AnyObject>::None];

        let next = Retained::into_raw(source);
        let previous = CURRENT_SOURCE.swap(next, Ordering::AcqRel);
        if let Some(previous) = Retained::from_raw(previous) {
            drop(previous);
        }
    }
    Ok(())
}
