// @author kongweiguang

//! Registry opening and probe single-flight operations.
//!
//! The service methods remain one implementation so registry ownership and
//! watcher installation continue to share the same synchronization boundaries.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gmark_document_core::{DocumentBackendKind, LoadingPolicy};
use gmark_document_runtime::{
    ControllerError, DocumentHandle, DocumentId, DocumentLease, DocumentRegistry,
    DocumentRegistryKey, DocumentSession, FileIdentity, RegistryOpen, SaveAsReserveOutcome,
    SaveStateNotification,
};
use gmark_paged_document::{OpenProbe, OpenStrategy, PreparedUtf8Source, ProbeOptions};
use gpui::{App, Global};

use crate::editor::services::file_watch::start_file_watch;

use super::runtime::{
    build_controller, build_host_controller, display_encoding, file_key, map_registry_error,
    normalize_path, registry_key_path,
};
use super::source::ResidentMarkdownSource;
use super::types::{
    DocumentSaveAsReservation, DocumentServiceError, SaveAsTargetReservation,
    SharedDocumentHostOpen, SharedResidentOpen, SharedSaveAsTarget,
};
use super::watcher::{
    ProbeKey, ProbeSlot, ProbeSlotState, WatcherControl, WatcherEntry, WatcherRegistration,
};
use super::watcher_runtime::{
    clear_probe_entries, remove_watcher, run_watcher, watcher_registration_parts,
};

/// 后台打开等待的上限；超过后让调用方恢复 UI 控制权，迟到结果由代次丢弃。
pub(super) const OPEN_WAIT_DEADLINE: Duration = Duration::from_secs(30);

#[path = "service_probe.rs"]
mod service_probe;

/// Process-wide owner of the application document registry.
#[derive(Clone)]
pub(crate) struct DocumentService {
    registry: Arc<DocumentRegistry>,
    watchers: Arc<Mutex<BTreeMap<DocumentRegistryKey, WatcherEntry>>>,
    probes: Arc<Mutex<BTreeMap<ProbeKey, Arc<ProbeSlot>>>>,
}
impl Global for DocumentService {}

impl Default for DocumentService {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentService {
    pub(crate) fn new() -> Self {
        Self {
            registry: Arc::new(DocumentRegistry::default()),
            watchers: Arc::new(Mutex::new(BTreeMap::new())),
            probes: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub(crate) fn init(cx: &mut App) {
        cx.set_global(Self::new());
    }

    /// Returns the process-global registry.  Callers should retain the
    /// returned `Arc` only for the duration of their adapter operation.
    pub(crate) fn registry(cx: &App) -> Arc<DocumentRegistry> {
        cx.global::<Self>().registry.clone()
    }

    pub(crate) fn registry_arc(&self) -> Arc<DocumentRegistry> {
        self.registry.clone()
    }

    /// Reserve a normalized Save As target while retaining service ownership
    /// of the source watcher.  Dropping the returned value releases the
    /// runtime reservation without changing the source key or watcher.
    pub(crate) fn reserve_save_as_target(
        &self,
        source: &DocumentHandle,
        path: impl AsRef<Path>,
    ) -> Result<SaveAsTargetReservation, DocumentServiceError> {
        let target_path = normalize_path(path.as_ref())?;
        let target_len = std::fs::metadata(&target_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let target_identity = FileIdentity {
            canonical_path: target_path.clone(),
            len: target_len,
            modified_nanos: None,
            platform_id: None,
        };
        let target_key = DocumentRegistryKey::for_file(&target_identity);
        let source_identity = source
            .lock()
            .map_err(map_registry_error)?
            .session()
            .file_identity
            .clone();
        let source_key = DocumentRegistryKey::for_file(&source_identity);
        let outcome = self
            .registry
            .reserve_save_as_outcome(source, target_key.clone())
            .map_err(map_registry_error)?;
        match outcome {
            SaveAsReserveOutcome::Reserved(reservation) => Ok(SaveAsTargetReservation::Reserved(
                DocumentSaveAsReservation {
                    service: self.clone(),
                    reservation: Some(reservation),
                    source_key,
                    target_key,
                    target_path,
                },
            )),
            SaveAsReserveOutcome::Occupied { handle, lease } => {
                let metadata = handle.lock().map_err(map_registry_error)?;
                let document_id = metadata.document_id();
                let text_encoding = metadata.session().profile.encoding.clone();
                let encoding = display_encoding(&text_encoding);
                let loading_limits = metadata.session().loading_limits;
                let probe = self.probe_for_existing_target(&target_key, metadata.session());
                drop(metadata);
                Ok(SaveAsTargetReservation::Occupied(SharedSaveAsTarget {
                    lease,
                    document_id,
                    key: target_key,
                    file_path: target_path,
                    encoding,
                    text_encoding,
                    loading_limits,
                    probe,
                }))
            }
        }
    }

    fn probe_for_existing_target(
        &self,
        key: &DocumentRegistryKey,
        session: &DocumentSession,
    ) -> OpenProbe {
        if let Some(probe) = self.cached_probe(key) {
            return probe;
        }
        let identity = gmark_paged_document::FileIdentity {
            path: session.file_identity.canonical_path.clone(),
            len: session.file_identity.len,
            modified_nanos: session.file_identity.modified_nanos,
            os_file_id: None,
        };
        let strategy = if session.store.kind() == DocumentBackendKind::Paged {
            OpenStrategy::Paged
        } else {
            OpenStrategy::Resident
        };
        OpenProbe {
            len: session.profile.len,
            identity,
            options: ProbeOptions {
                max_resident_bytes: session.loading_limits.max_resident_bytes,
                ..ProbeOptions::default()
            },
            format: session.profile.format.clone(),
            encoding: session.profile.encoding.clone(),
            strategy,
            force_safe_source: strategy == OpenStrategy::Paged,
            estimated_lines: session.profile.estimated_lines,
            estimated_structural_units: session.profile.estimated_structural_units,
        }
    }

    /// Reuse a ready probe for an existing target so Save As does not perform
    /// another filesystem probe while the service already owns its result.
    // 原因：已有 registry 会话的元数据足以构造目标预览，复用缓存可避免重复阻塞 I/O。
    fn cached_probe(&self, key: &DocumentRegistryKey) -> Option<OpenProbe> {
        let probes = match self.probes.lock() {
            Ok(probes) => probes,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (probe_key, slot) in probes.iter() {
            if &probe_key.key != key {
                continue;
            }
            let state = match slot.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let ProbeSlotState::Ready(probe) = &*state {
                return Some(probe.clone());
            }
        }
        None
    }

    /// Drop a probe that was classified as Paged or whose resident open
    /// failed before a watcher could own the cache entry.
    pub(crate) fn clear_probe(&self, path: impl AsRef<Path>, policy: LoadingPolicy) {
        let Ok(normalized_path) = normalize_path(path.as_ref()) else {
            return;
        };
        let key = ProbeKey {
            key: file_key(&normalized_path),
            max_resident_bytes: policy.effective_limits().max_resident_bytes,
            force_safe_source: policy.force_safe_source,
        };
        let mut probes = match self.probes.lock() {
            Ok(probes) => probes,
            Err(poisoned) => poisoned.into_inner(),
        };
        probes.remove(&key);
    }

    /// Opens a resident Markdown file through one canonical registry key.
    ///
    /// Path normalization happens before touching the registry.  The loader is
    /// invoked only by the `Opening` owner and receives the normalized path and
    /// frozen loading policy; all file/decode work therefore remains outside
    /// registry, slot, and controller locks.
    pub(crate) fn open_resident_file<F, E>(
        &self,
        path: impl AsRef<Path>,
        policy: LoadingPolicy,
        loader: F,
    ) -> Result<SharedResidentOpen, DocumentServiceError>
    where
        F: FnOnce(&Path, LoadingPolicy) -> Result<ResidentMarkdownSource, E>,
        E: std::fmt::Display,
    {
        let normalized_path = normalize_path(path.as_ref())?;
        let key = file_key(&normalized_path);
        let loader_path = normalized_path.clone();
        let result = self.registry.open_or_insert_leased(key.clone(), || {
            let mut source = loader(&loader_path, policy)
                .map_err(|error| ControllerError::open_failed(error.to_string()))?;
            source.loading_limits = policy.effective_limits();
            // 注册表键负责折叠 Windows 路径别名，文件身份则必须保留探测器返回的
            // canonical path；把后者改写成 dunce 路径会让首次保存误判为外部修改。
            build_controller(DocumentId::new(), source, false)
        });
        self.finish_open(key, result)
    }

    /// Opens a ResidentFormat or Paged document through one canonical
    /// controller.  The loader owns all source preparation and runs only in
    /// the registry Opening owner; registry/controller locks remain untouched
    /// while it performs filesystem, decoding, shadow and indexing work.
    pub(crate) fn open_document_host<F, E>(
        &self,
        path: impl AsRef<Path>,
        probe: OpenProbe,
        policy: LoadingPolicy,
        loader: F,
    ) -> Result<SharedDocumentHostOpen, DocumentServiceError>
    where
        F: FnOnce(&Path, &OpenProbe, LoadingPolicy) -> Result<PreparedUtf8Source, E>,
        E: std::fmt::Display,
    {
        let normalized_path = normalize_path(path.as_ref())?;
        let key = file_key(&normalized_path);
        let loader_path = normalized_path.clone();
        let owner_probe = probe.clone();
        let result = self.registry.open_or_insert_leased(key.clone(), || {
            let prepared = loader(&loader_path, &owner_probe, policy)
                .map_err(|error| ControllerError::open_failed(error.to_string()))?;
            build_host_controller(DocumentId::new(), owner_probe, policy, prepared)
        });
        self.finish_host_open(key, probe, result)
    }

    /// Paged-only shorthand used by large-document windows.  Keeping this
    /// check at the service boundary prevents a Paged host from accidentally
    /// publishing a Resident Controller when a stale probe is supplied.
    pub(crate) fn open_paged<F, E>(
        &self,
        path: impl AsRef<Path>,
        probe: OpenProbe,
        policy: LoadingPolicy,
        loader: F,
    ) -> Result<SharedDocumentHostOpen, DocumentServiceError>
    where
        F: FnOnce(&Path, &OpenProbe, LoadingPolicy) -> Result<PreparedUtf8Source, E>,
        E: std::fmt::Display,
    {
        if probe.strategy != OpenStrategy::Paged {
            return Err(DocumentServiceError::OpenFailed(
                "paged host open received a resident probe".to_owned(),
            ));
        }
        self.open_document_host(path, probe, policy, loader)
    }

    /// Registers an untitled Markdown source under a stable UUID.  Passing
    /// `None` creates a fresh identity; recovery and tab restore can pass an
    /// existing UUID to rejoin the same process-local document.
    pub(crate) fn open_untitled(
        &self,
        document_id: Option<DocumentId>,
        source: ResidentMarkdownSource,
    ) -> Result<SharedResidentOpen, DocumentServiceError> {
        let document_id = document_id.unwrap_or_default();
        let key = DocumentRegistryKey::Untitled(document_id);
        let result = self
            .registry
            .open_or_insert_leased(key.clone(), || build_controller(document_id, source, false));
        self.finish_open(key, result)
    }

    /// Registers a recovery source under the recovery UUID while keeping its
    /// source body in the shared runtime session.  Recovery content starts
    /// dirty so the normal save/recovery lifecycle can observe it.
    pub(crate) fn open_recovery(
        &self,
        document_id: DocumentId,
        source: ResidentMarkdownSource,
    ) -> Result<SharedResidentOpen, DocumentServiceError> {
        let key = DocumentRegistryKey::Untitled(document_id);
        let result = self
            .registry
            .open_or_insert_leased(key.clone(), || build_controller(document_id, source, true));
        self.finish_open(key, result)
    }

    fn finish_open(
        &self,
        key: DocumentRegistryKey,
        result: Result<(DocumentHandle, DocumentLease, RegistryOpen), ControllerError>,
    ) -> Result<SharedResidentOpen, DocumentServiceError> {
        let (handle, lease, open) = result.map_err(map_registry_error)?;
        let metadata = handle.lock().map_err(map_registry_error)?;
        let document_id = metadata.document_id();
        let text_encoding = metadata.session().profile.encoding.clone();
        let encoding = display_encoding(&text_encoding);
        let loading_limits = metadata.session().loading_limits;
        let file_path = registry_key_path(&key);
        drop(metadata);
        let shared = SharedResidentOpen {
            lease,
            open,
            document_id,
            encoding,
            text_encoding,
            loading_limits,
            key,
            file_path,
        };
        if shared.open == RegistryOpen::Inserted {
            self.start_watcher(&shared);
        }
        Ok(shared)
    }

    fn finish_host_open(
        &self,
        key: DocumentRegistryKey,
        probe: OpenProbe,
        result: Result<(DocumentHandle, DocumentLease, RegistryOpen), ControllerError>,
    ) -> Result<SharedDocumentHostOpen, DocumentServiceError> {
        let (handle, lease, open) = result.map_err(map_registry_error)?;
        let metadata = handle.lock().map_err(map_registry_error)?;
        let document_id = metadata.document_id();
        let text_encoding = metadata.session().profile.encoding.clone();
        let encoding = display_encoding(&text_encoding);
        let loading_limits = metadata.session().loading_limits;
        let file_path = registry_key_path(&key);
        drop(metadata);
        let shared = SharedDocumentHostOpen {
            lease,
            open,
            probe,
            document_id,
            encoding,
            text_encoding,
            loading_limits,
            key,
            file_path,
        };
        if shared.open == RegistryOpen::Inserted {
            self.start_watcher_for_host(&shared);
        }
        Ok(shared)
    }

    fn start_watcher_for_host(&self, shared: &SharedDocumentHostOpen) {
        // Host sessions use the same process watcher implementation as
        // resident Markdown.  Keep this narrow wrapper so future host-only
        // cleanup can diverge without constructing a second watcher here.
        self.start_watcher_key(
            shared.key.clone(),
            shared.handle(),
            Some(shared.probe.clone()),
        );
    }

    /// Start one debounced watcher for the newly inserted Controller.  The
    /// watch guard lives on the worker thread and exits once the last lease is
    /// released; a subsequent open can then install a fresh entry safely.
    fn start_watcher(&self, shared: &SharedResidentOpen) {
        self.start_watcher_key(shared.key.clone(), shared.handle(), None);
    }

    fn start_watcher_key(
        &self,
        key: DocumentRegistryKey,
        handle: DocumentHandle,
        probe: Option<OpenProbe>,
    ) {
        let path = match handle.lock() {
            Ok(controller) => controller.session().file_identity.canonical_path.clone(),
            Err(error) => {
                eprintln!("failed to inspect document watcher identity: {error}");
                return;
            }
        };
        if path.as_os_str().is_empty() {
            return;
        }

        let (guard, receiver) = match start_file_watch(path.clone()) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("failed to watch '{}': {error}", path.display());
                clear_probe_entries(&Arc::downgrade(&self.probes), &key);
                return;
            }
        };

        let (save_sender, save_receiver) = futures::channel::mpsc::unbounded();
        let save_registration = match handle.register_save_state_callback(Arc::new(
            move |notification: SaveStateNotification| {
                let _ = save_sender.unbounded_send(notification);
            },
        )) {
            Ok(registration) => registration,
            Err(error) => {
                eprintln!("failed to register save-state watcher callback: {error}");
                drop(guard);
                return;
            }
        };
        let watchers = Arc::downgrade(&self.watchers);
        let probes = Arc::downgrade(&self.probes);
        let control = Arc::new(WatcherControl {
            guard: Mutex::new(Some(guard)),
            save_registration: Mutex::new(Some(save_registration)),
            pending_changed: std::sync::atomic::AtomicBool::new(false),
        });
        let token = Arc::new(());
        let registration = Arc::new(Mutex::new((
            key.clone(),
            Arc::clone(&token),
            Arc::downgrade(&control),
        )));
        {
            let mut watchers = match self.watchers.lock() {
                Ok(watchers) => watchers,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(existing) = watchers.get(&key)
                && existing.handle.lease_count() != 0
            {
                control.stop();
                return;
            }
            watchers.insert(
                key.clone(),
                WatcherEntry {
                    handle: handle.clone(),
                    registration: Arc::clone(&registration),
                    control: Arc::clone(&control),
                },
            );
        }

        let callback_watchers = watchers.clone();
        let callback_probes = probes.clone();
        let callback_registration = Arc::clone(&registration);
        if let Err(error) = handle.register_last_lease_callback(Arc::new(move || {
            let (callback_key, callback_token, callback_control) =
                watcher_registration_parts(&callback_registration);
            let removed = remove_watcher(&callback_watchers, &callback_key, &callback_token);
            if removed {
                clear_probe_entries(&callback_probes, &callback_key);
            }
            if let Some(control) = callback_control.upgrade() {
                control.stop();
            }
        })) {
            eprintln!("failed to register watcher cleanup callback: {error}");
            remove_watcher(&watchers, &key, &token);
            clear_probe_entries(&probes, &key);
            control.stop();
            return;
        }
        thread::spawn(move || {
            futures::executor::block_on(run_watcher(
                control,
                receiver,
                save_receiver,
                path,
                key,
                handle,
                probe,
                token,
                watchers,
                probes,
            ));
        });
    }

    pub(super) fn rekey_watcher_after_save_as(
        &self,
        source_key: &DocumentRegistryKey,
        target_key: &DocumentRegistryKey,
        target_path: &Path,
        handle: &DocumentHandle,
    ) {
        let old_entry = {
            let mut watchers = match self.watchers.lock() {
                Ok(watchers) => watchers,
                Err(poisoned) => poisoned.into_inner(),
            };
            watchers.remove(source_key)
        };
        let Some(old_entry) = old_entry else {
            return;
        };
        old_entry.control.stop();
        clear_probe_entries(&Arc::downgrade(&self.probes), source_key);
        self.start_rekeyed_watcher(
            target_key.clone(),
            target_path.to_path_buf(),
            handle.clone(),
            old_entry.registration,
        );
    }

    fn start_rekeyed_watcher(
        &self,
        key: DocumentRegistryKey,
        path: PathBuf,
        handle: DocumentHandle,
        registration: WatcherRegistration,
    ) {
        let (guard, receiver) = match start_file_watch(path.clone()) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("failed to watch '{}': {error}", path.display());
                clear_probe_entries(&Arc::downgrade(&self.probes), &key);
                return;
            }
        };
        let (save_sender, save_receiver) = futures::channel::mpsc::unbounded();
        let save_registration = match handle.register_save_state_callback(Arc::new(
            move |notification: SaveStateNotification| {
                let _ = save_sender.unbounded_send(notification);
            },
        )) {
            Ok(registration) => registration,
            Err(error) => {
                eprintln!("failed to register save-state watcher callback: {error}");
                drop(guard);
                return;
            }
        };
        let control = Arc::new(WatcherControl {
            guard: Mutex::new(Some(guard)),
            save_registration: Mutex::new(Some(save_registration)),
            pending_changed: std::sync::atomic::AtomicBool::new(false),
        });
        let token = Arc::new(());
        {
            let mut state = match registration.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            *state = (key.clone(), Arc::clone(&token), Arc::downgrade(&control));
        }
        {
            let mut watchers = match self.watchers.lock() {
                Ok(watchers) => watchers,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(existing) = watchers.get(&key)
                && existing.handle.lease_count() != 0
            {
                control.stop();
                return;
            }
            watchers.insert(
                key.clone(),
                WatcherEntry {
                    handle: handle.clone(),
                    registration,
                    control: Arc::clone(&control),
                },
            );
        }
        let watchers = Arc::downgrade(&self.watchers);
        let probes = Arc::downgrade(&self.probes);
        thread::spawn(move || {
            futures::executor::block_on(run_watcher(
                control,
                receiver,
                save_receiver,
                path,
                key,
                handle,
                None,
                token,
                watchers,
                probes,
            ));
        });
    }

    #[cfg(test)]
    pub(crate) fn watcher_count(&self) -> usize {
        match self.watchers.lock() {
            Ok(watchers) => watchers.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_watcher_count(&self) -> usize {
        match self.watchers.lock() {
            Ok(watchers) => watchers
                .values()
                .filter(|entry| {
                    entry
                        .control
                        .pending_changed
                        .load(std::sync::atomic::Ordering::Acquire)
                })
                .count(),
            Err(poisoned) => poisoned
                .into_inner()
                .values()
                .filter(|entry| {
                    entry
                        .control
                        .pending_changed
                        .load(std::sync::atomic::Ordering::Acquire)
                })
                .count(),
        }
    }
}
