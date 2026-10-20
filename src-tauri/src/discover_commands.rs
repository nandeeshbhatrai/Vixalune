//! IPC for Discover and the community catalog. The interface sends a search text or a package id; the backend validates it, reads
//! the package's facts itself, and generates the catalog entry. Adding an entry installs nothing.

use crate::commands::{log, AppState};
use crate::domain::activity::ActivityKind;
use crate::domain::discover::{DiscoverResult, DiscoveredDetails};
use crate::domain::process::CancellationToken;
use crate::domain::AppResult;
use crate::ports::CatalogEntry;
use tauri::State;

#[tauri::command]
pub async fn discover_search(state: State<'_, AppState>, query: String) -> AppResult<Vec<DiscoverResult>> {
    let community = state.community.clone();
    community.search(&query, &CancellationToken::new()).await
}

#[tauri::command]
pub async fn discover_details(state: State<'_, AppState>, id: String) -> AppResult<DiscoveredDetails> {
    let community = state.community.clone();
    community.details(&id, &CancellationToken::new()).await
}

/// Add a package to the catalog as a community entry (not installed). Install is a separate step with its own confirmation.
#[tauri::command]
pub async fn discover_add(state: State<'_, AppState>, id: String) -> AppResult<CatalogEntry> {
    let community = state.community.clone();
    let entry = community.add(&id, &CancellationToken::new()).await?;
    log(&state, ActivityKind::SettingsChanged, &format!("added community package {} to the catalog (not installed)", entry.manifest.id));
    Ok(entry)
}

#[tauri::command]
pub fn community_remove(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.community.remove(&id)?;
    log(&state, ActivityKind::SettingsChanged, &format!("removed community entry {id} from the catalog"));
    Ok(())
}
