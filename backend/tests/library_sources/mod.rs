//! The library's sources for a test, set through the app's commands.

use backend::commands::BackendCommands;
use backend::models::{
    AddSourceFoldersRequest, ExternalLibrary, SetExternalLibraryEnabledRequest,
    SetSourceFolderEnabledRequest, SourceFolder,
};
use backend::service::BackendService;

/// The commands `show_only` uses, from either entry point a test holds.
pub trait SourcesApi {
    fn add_folders(&self, paths: Vec<String>) -> Vec<SourceFolder>;
    fn switch_folder(&self, path: String, enabled: bool);
    fn switch_library(&self, library: ExternalLibrary, enabled: bool);
}

impl<T: SourcesApi> SourcesApi for &T {
    fn add_folders(&self, paths: Vec<String>) -> Vec<SourceFolder> {
        (**self).add_folders(paths)
    }

    fn switch_folder(&self, path: String, enabled: bool) {
        (**self).switch_folder(path, enabled)
    }

    fn switch_library(&self, library: ExternalLibrary, enabled: bool) {
        (**self).switch_library(library, enabled)
    }
}

impl SourcesApi for BackendCommands {
    fn add_folders(&self, paths: Vec<String>) -> Vec<SourceFolder> {
        let added = self.add_source_folders(AddSourceFoldersRequest { paths });
        added.data.expect("add source folders").sources.folders
    }

    fn switch_folder(&self, path: String, enabled: bool) {
        let switched =
            self.set_source_folder_enabled(SetSourceFolderEnabledRequest { path, enabled });
        assert!(switched.ok, "switch source folder: {switched:?}");
    }

    fn switch_library(&self, library: ExternalLibrary, enabled: bool) {
        let switched = self
            .set_external_library_enabled(SetExternalLibraryEnabledRequest { library, enabled });
        assert!(switched.ok, "switch library: {switched:?}");
    }
}

impl SourcesApi for BackendService {
    fn add_folders(&self, paths: Vec<String>) -> Vec<SourceFolder> {
        self.add_source_folders(AddSourceFoldersRequest { paths })
            .expect("add source folders")
            .sources
            .folders
    }

    fn switch_folder(&self, path: String, enabled: bool) {
        self.set_source_folder_enabled(SetSourceFolderEnabledRequest { path, enabled })
            .expect("switch source folder");
    }

    fn switch_library(&self, library: ExternalLibrary, enabled: bool) {
        self.set_external_library_enabled(SetExternalLibraryEnabledRequest { library, enabled })
            .expect("switch library");
    }
}

/// The library shows exactly `roots` (added if new) and the imported
/// libraries as given. Every other configured folder is switched off, not
/// removed: removing a folder drops its tracks.
pub fn show_only(api: &impl SourcesApi, roots: Vec<String>, rekordbox_db: bool, mixxx_db: bool) {
    let roots: Vec<String> = roots.iter().map(|root| root.trim().to_string()).collect();
    for folder in api.add_folders(roots.clone()) {
        let enabled = roots.contains(&folder.path);
        if folder.enabled != enabled {
            api.switch_folder(folder.path, enabled);
        }
    }
    api.switch_library(ExternalLibrary::Rekordbox, rekordbox_db);
    api.switch_library(ExternalLibrary::Mixxx, mixxx_db);
}
