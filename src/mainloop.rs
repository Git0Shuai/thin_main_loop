#[cfg(feature = "glib")]
use crate::glib::Backend;

#[cfg(feature = "win32")]
use crate::winmsg::Backend;

#[cfg(not(any(feature = "win32", feature = "glib")))]
use crate::ruststd::Backend;

use std::cell::Cell;
use std::ptr::NonNull;
use std::marker::PhantomData;
use std::rc::Rc;
use std::panic;
use std::time::Duration;
use crate::{CbKind, CbId, MainLoopError};

pub (crate) fn call_internal(cb: CbKind<'static>) -> Result<CbId, MainLoopError> {
    current_loop.with(|ml| {
        let ml = ml.get().ok_or(MainLoopError::NoMainLoop)?;
        let ml = unsafe { ml.as_ref() };
        ml.backend.push(cb)
    })
}

pub (crate) fn terminate() {
    current_loop.with(|ml| {
        if let Some(ml) = ml.get() { 
            let ml = unsafe { ml.as_ref() };
            ml.quit(); 
        }
    });
}

thread_local! {
    static current_loop: Cell<Option<NonNull<MainLoop<'static>>>> = Default::default();
}



pub struct MainLoop<'a> {
    terminated: Cell<bool>,
    backend: Backend<'a>,
    _z: PhantomData<Rc<()>>, // !Send, !Sync
}

impl<'a> MainLoop<'a> {
    pub fn quit(&self) { self.terminated.set(true) }
    pub fn call_asap<F: FnOnce() + 'a>(&self, f: F) -> Result<CbId, MainLoopError> {
        self.backend.push(CbKind::asap(f))
    }
    pub fn call_after<F: FnOnce() + 'a>(&self, d: Duration, f: F) -> Result<CbId, MainLoopError> { 
        self.backend.push(CbKind::after(f, d))
    }
    pub fn call_interval<F: FnMut() -> bool + 'a>(&self, d: Duration, f: F)  -> Result<CbId, MainLoopError> {
        self.backend.push(CbKind::interval(f, d))
    }

    fn with_current_loop<F: FnOnce()>(&self, f: F) {
        if self.terminated.get() { return; }
        current_loop.with(|ml| {
            if ml.get().is_some() { panic!("Reentrant call to MainLoop") }
            ml.set(Some(NonNull::from(self).cast()));
        });
        let r = panic::catch_unwind(panic::AssertUnwindSafe(|| {
             f()
        }));
        current_loop.with(|ml| { ml.set(None); });
        if let Err(e) = r { panic::resume_unwind(e) };
    }

    /// Runs the main loop until terminated.
    pub fn run(&mut self) {
        self.with_current_loop(|| {
            while !self.terminated.get() {
                self.backend.run_one(true);
            }
        })
    }

    /// Runs the main loop once, without waiting.
    pub fn run_one(&mut self) {
        self.with_current_loop(|| {
            if !self.terminated.get() {
                self.backend.run_one(false);
            }
        })
    }

    /// Creates a new main loop
    pub fn new() -> Self { MainLoop { 
        terminated: Cell::new(false),
        backend: Backend::new(),
        _z: PhantomData 
    } }
}


#[test]
fn borrowed() {
    let mut x;
    {
        let mut ml = MainLoop::new();
        x = false;
        ml.call_asap(|| { x = true; terminate(); }).unwrap();
        ml.run();
    }
    assert_eq!(x, true);
}

#[test]
fn asap_static() {
    use std::rc::Rc;

    let x;
    let mut ml = MainLoop::new();
    x = Rc::new(Cell::new(0));
    let xcl = x.clone();
    ml.call_asap(|| { 
        assert_eq!(x.get(), 0);
        x.set(1);
        crate::call_asap(move || {
            assert_eq!(xcl.get(), 1);
            xcl.set(2);
            terminate();
        }).unwrap();
    }).unwrap();
    ml.run();
    assert_eq!(x.get(), 2);
}

#[test]
fn after() {
    use std::time::Instant;
    let x;
    let mut ml = MainLoop::new();
    x = Cell::new(false);
    let n = Instant::now();
    ml.call_after(Duration::from_millis(300), || { x.set(true); terminate(); }).unwrap();
    ml.run();
    assert_eq!(x.get(), true);
    assert!(Instant::now() - n >= Duration::from_millis(300)); 
}

#[test]
fn interval() {
    use std::time::Instant;
    let mut x = 0;
    let mut y = 0;
    let n = Instant::now();
    {
        let mut ml = MainLoop::new();
        ml.call_interval(Duration::from_millis(150), || {
            y += 1;
            false
        }).unwrap();
        ml.call_interval(Duration::from_millis(100), || {
           println!("{}", x);
           x += 1;
           if x >= 4 { terminate(); }
           true
        }).unwrap();
        ml.run();
    }
    assert_eq!(y, 1);
    assert_eq!(x, 4);
    assert!(Instant::now() - n >= Duration::from_millis(400)); 
}