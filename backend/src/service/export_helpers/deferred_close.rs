//! Closes written export files on background threads.
//!
//! Linux desktops mount FAT sticks with the `flush` option, and the vfat
//! driver then sleeps ~100 ms in `close()` of every file opened for writing
//! (`fat_file_release`). An export writes several files per track (audio,
//! analysis bundle, artwork), so closing them inline on the export thread
//! spends most of the run asleep while the stick sits idle. Handing the open
//! `File` to a small pool lets those sleeps overlap with each other and with
//! the next track's work. Data is already in the page cache when a handle is
//! handed over, so readers see it immediately; `finish` is the barrier that
//! waits for every close before the databases are written.

use std::fs::File;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Closer threads; each spends ~100 ms per close on a `flush` FAT mount.
const CLOSER_THREADS: usize = 16;
/// Handles queued beyond this block the sender (bounds open descriptors).
const CLOSER_QUEUE: usize = 64;

pub struct FileCloser {
    tx: Option<SyncSender<File>>,
    workers: Vec<JoinHandle<()>>,
    /// Handed-over files not yet closed.
    pending: Arc<AtomicUsize>,
}

impl FileCloser {
    pub fn new() -> Self {
        let (tx, rx) = sync_channel::<File>(CLOSER_QUEUE);
        let rx = Arc::new(Mutex::new(rx));
        let pending = Arc::new(AtomicUsize::new(0));
        let workers = (0..CLOSER_THREADS)
            .map(|_| {
                let rx = Arc::clone(&rx);
                let pending = Arc::clone(&pending);
                std::thread::spawn(move || close_until_disconnected(&rx, &pending))
            })
            .collect();
        Self {
            tx: Some(tx),
            workers,
            pending,
        }
    }

    /// Closes `file` on a pool thread (inline if the pool is gone).
    pub fn close(&self, file: File) {
        let Some(tx) = &self.tx else {
            return;
        };
        self.pending.fetch_add(1, Ordering::SeqCst);
        if let Err(err) = tx.send(file) {
            drop(err.0);
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Whether any handed-over file is still waiting to be closed.
    pub fn has_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }

    /// Waits until every handed-over file has been closed.
    pub fn finish(mut self) {
        self.join_workers();
    }

    fn join_workers(&mut self) {
        self.tx = None;
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Default for FileCloser {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FileCloser {
    fn drop(&mut self) {
        self.join_workers();
    }
}

fn close_until_disconnected(rx: &Mutex<Receiver<File>>, pending: &AtomicUsize) {
    loop {
        let next = match rx.lock() {
            Ok(rx) => rx.recv(),
            Err(_) => return,
        };
        match next {
            Ok(file) => {
                drop(file);
                pending.fetch_sub(1, Ordering::SeqCst);
            }
            Err(_) => return,
        }
    }
}
