//! C ABI session handle — local `RefCell` or shared `Arc` for embedded agent HTTP (#210).

#![allow(unsafe_code)]

use std::cell::{RefCell, RefMut};
use std::sync::Arc;

use parking_lot::Mutex as ParkingMutex;

use crate::session::{HostSession, ModelId};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SessionInner {
    Local(RefCell<HostSession>),
    Shared(Arc<ParkingMutex<HostSession>>),
}

#[derive(Debug)]
pub struct SessionHandle {
    pub inner: SessionInner,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum SessionAccess<'a> {
    Local(RefMut<'a, HostSession>),
    Shared(parking_lot::MutexGuard<'a, HostSession>),
}

impl std::ops::Deref for SessionAccess<'_> {
    type Target = HostSession;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Local(r) => r,
            Self::Shared(g) => g,
        }
    }
}

impl std::ops::DerefMut for SessionAccess<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Local(r) => r,
            Self::Shared(g) => g,
        }
    }
}

fn handle_mut(handle: *mut std::ffi::c_void) -> Option<&'static mut SessionHandle> {
    if handle.is_null() {
        None
    } else {
        // SAFETY: opaque handle from `sc_create`; callers serialize per-handle access.
        Some(unsafe { &mut *(handle.cast::<SessionHandle>()) })
    }
}

pub fn session_access(handle: *mut std::ffi::c_void) -> Option<SessionAccess<'static>> {
    let h = handle_mut(handle)?;
    Some(match &mut h.inner {
        SessionInner::Local(c) => SessionAccess::Local(c.borrow_mut()),
        SessionInner::Shared(a) => SessionAccess::Shared(a.lock()),
    })
}

/// Promote to shared storage for embedded agent HTTP (idempotent).
pub fn share_session_arc(handle: *mut std::ffi::c_void) -> Option<Arc<ParkingMutex<HostSession>>> {
    let h = handle_mut(handle)?;
    match &mut h.inner {
        SessionInner::Shared(a) => Some(Arc::clone(a)),
        SessionInner::Local(_) => {
            let placeholder = HostSession::new(ModelId::Spectrum48, true);
            let SessionInner::Local(cell) =
                std::mem::replace(&mut h.inner, SessionInner::Local(RefCell::new(placeholder)))
            else {
                unreachable!("just checked Local");
            };
            let arc = Arc::new(ParkingMutex::new(cell.into_inner()));
            h.inner = SessionInner::Shared(Arc::clone(&arc));
            Some(arc)
        }
    }
}
