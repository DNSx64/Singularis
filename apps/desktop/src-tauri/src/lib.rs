#![forbid(unsafe_code)]

mod audio;
mod messaging;
mod outbox;
mod video;

use std::sync::{Arc, Mutex, MutexGuard};

use audio::{AudioError, AudioState, AudioStatus};
use messaging::MessagingError;
use outbox::{OutboxWorker, OutboxWorkerStatus};
use serde::Serialize;
use singularis_vault::{NewVaultMessage, Vault, VaultError, VaultMessage, VaultStatus};
use tauri::{Manager, State};
use video::{VideoState, VideoStatus};
use zeroize::Zeroizing;

type ManagedVault = Arc<Mutex<Vault>>;
type ManagedOutboxWorker = Arc<OutboxWorker>;
type ManagedAudioState = Arc<Mutex<AudioState>>;
type ManagedVideoState = Arc<Mutex<VideoState>>;
type CommandResult<T> = std::result::Result<T, CommandError>;

#[derive(Clone, Debug, Serialize)]
struct CommandError {
    code: &'static str,
    message: &'static str,
}

impl From<VaultError> for CommandError {
    fn from(error: VaultError) -> Self {
        Self {
            code: error.code(),
            message: error.public_message(),
        }
    }
}

impl From<MessagingError> for CommandError {
    fn from(error: MessagingError) -> Self {
        Self {
            code: error.code(),
            message: error.public_message(),
        }
    }
}

impl From<AudioError> for CommandError {
    fn from(error: AudioError) -> Self {
        Self {
            code: error.code(),
            message: error.public_message(),
        }
    }
}

#[tauri::command]
fn vault_status(state: State<'_, ManagedVault>) -> CommandResult<VaultStatus> {
    lock_vault(state.inner())?.status().map_err(Into::into)
}

#[tauri::command]
async fn vault_initialize(
    state: State<'_, ManagedVault>,
    outbox: State<'_, ManagedOutboxWorker>,
    passphrase: String,
) -> CommandResult<VaultStatus> {
    let vault = Arc::clone(state.inner());
    let status = tauri::async_runtime::spawn_blocking(move || {
        let passphrase = Zeroizing::new(passphrase);
        lock_vault(&vault)?
            .initialize(passphrase.as_str())
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| internal_command_error())??;
    outbox.resume();
    Ok(status)
}

#[tauri::command]
async fn vault_unlock(
    state: State<'_, ManagedVault>,
    outbox: State<'_, ManagedOutboxWorker>,
    passphrase: String,
) -> CommandResult<VaultStatus> {
    let vault = Arc::clone(state.inner());
    let status = tauri::async_runtime::spawn_blocking(move || {
        let passphrase = Zeroizing::new(passphrase);
        lock_vault(&vault)?
            .unlock(passphrase.as_str())
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| internal_command_error())??;
    outbox.resume();
    Ok(status)
}

#[tauri::command]
async fn vault_lock(
    state: State<'_, ManagedVault>,
    outbox: State<'_, ManagedOutboxWorker>,
) -> CommandResult<VaultStatus> {
    outbox.pause_and_wait().await;
    let vault = Arc::clone(state.inner());
    let result = match tauri::async_runtime::spawn_blocking(move || {
        lock_vault(&vault)?.lock().map_err(CommandError::from)
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Err(internal_command_error()),
    };
    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            outbox.resume();
            Err(error)
        }
    }
}

#[tauri::command]
fn outbox_status(state: State<'_, ManagedOutboxWorker>) -> OutboxWorkerStatus {
    state.status()
}

#[tauri::command]
async fn outbox_retry(
    vault: State<'_, ManagedVault>,
    state: State<'_, ManagedOutboxWorker>,
) -> CommandResult<OutboxWorkerStatus> {
    Ok(state.flush_now(vault.inner()).await)
}

#[tauri::command]
fn audio_status(state: State<'_, ManagedAudioState>) -> CommandResult<AudioStatus> {
    let audio = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(audio.status())
}

#[tauri::command]
fn audio_set_muted(state: State<'_, ManagedAudioState>, muted: bool) -> CommandResult<AudioStatus> {
    let mut audio = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(audio.set_muted(muted))
}

#[tauri::command]
fn audio_set_deafened(
    state: State<'_, ManagedAudioState>,
    deafened: bool,
) -> CommandResult<AudioStatus> {
    let mut audio = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(audio.set_deafened(deafened))
}

#[tauri::command]
fn audio_set_push_to_talk(
    state: State<'_, ManagedAudioState>,
    enabled: bool,
) -> CommandResult<AudioStatus> {
    let mut audio = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(audio.set_push_to_talk(enabled))
}

#[tauri::command]
fn audio_set_ptt_pressed(
    state: State<'_, ManagedAudioState>,
    pressed: bool,
) -> CommandResult<AudioStatus> {
    let mut audio = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(audio.set_ptt_pressed(pressed))
}

#[tauri::command]
fn audio_join_room(
    state: State<'_, ManagedAudioState>,
    room_id: String,
) -> CommandResult<AudioStatus> {
    let mut audio = state.inner().lock().map_err(|_| internal_command_error())?;
    audio.join_room(&room_id).map_err(Into::into)
}

#[tauri::command]
fn audio_leave_room(state: State<'_, ManagedAudioState>) -> CommandResult<AudioStatus> {
    let mut audio = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(audio.leave_room())
}

#[tauri::command]
fn video_status(state: State<'_, ManagedVideoState>) -> CommandResult<VideoStatus> {
    let video = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(video.status())
}

#[tauri::command]
fn video_set_camera_enabled(
    state: State<'_, ManagedVideoState>,
    enabled: bool,
) -> CommandResult<VideoStatus> {
    let mut video = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(video.set_camera_enabled(enabled))
}

#[tauri::command]
fn video_set_screen_share_enabled(
    state: State<'_, ManagedVideoState>,
    enabled: bool,
) -> CommandResult<VideoStatus> {
    let mut video = state.inner().lock().map_err(|_| internal_command_error())?;
    Ok(video.set_screen_share_enabled(enabled))
}

#[tauri::command]
async fn vault_queue_message(
    state: State<'_, ManagedVault>,
    outbox: State<'_, ManagedOutboxWorker>,
    message: NewVaultMessage,
) -> CommandResult<VaultMessage> {
    let vault = Arc::clone(state.inner());
    let stored = tauri::async_runtime::spawn_blocking(move || {
        let vault = lock_vault(&vault)?;
        messaging::queue_message(&vault, &message).map_err(CommandError::from)
    })
    .await
    .map_err(|_| internal_command_error())??;
    outbox.notify_pending();
    Ok(stored)
}

#[tauri::command]
fn vault_list_messages(
    state: State<'_, ManagedVault>,
    channel_id: String,
) -> CommandResult<Vec<VaultMessage>> {
    lock_vault(state.inner())?
        .list_messages(&channel_id)
        .map_err(Into::into)
}

#[tauri::command]
fn vault_search_messages(
    state: State<'_, ManagedVault>,
    channel_id: String,
    query: String,
) -> CommandResult<Vec<VaultMessage>> {
    lock_vault(state.inner())?
        .search_messages(&channel_id, &query)
        .map_err(Into::into)
}

fn lock_vault(vault: &Mutex<Vault>) -> CommandResult<MutexGuard<'_, Vault>> {
    vault.lock().map_err(|_| internal_command_error())
}

const fn internal_command_error() -> CommandError {
    CommandError {
        code: "internal_error",
        message: "Der lokale Vault konnte nicht verarbeitet werden.",
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let vault_root = app.path().app_data_dir()?.join("vault-v1");
            let vault = Arc::new(Mutex::new(Vault::new(vault_root)));
            let outbox = OutboxWorker::start(Arc::clone(&vault))?;
            let audio = Arc::new(Mutex::new(AudioState::new()));
            let video = Arc::new(Mutex::new(VideoState::new()));
            app.manage(vault);
            app.manage(outbox);
            app.manage(audio);
            app.manage(video);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            vault_initialize,
            vault_unlock,
            vault_lock,
            outbox_status,
            outbox_retry,
            audio_status,
            audio_set_muted,
            audio_set_deafened,
            audio_set_push_to_talk,
            audio_set_ptt_pressed,
            audio_join_room,
            audio_leave_room,
            video_status,
            video_set_camera_enabled,
            video_set_screen_share_enabled,
            vault_queue_message,
            vault_list_messages,
            vault_search_messages,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the Singularis desktop application");
}
