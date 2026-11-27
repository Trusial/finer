#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use finer_engine::{
    Conversion, Error, Event, FrameSize, Media, Output, PassCheckpoint, Settings, Tools,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio_util::sync::CancellationToken;

struct Application {
    data_directory: PathBuf,
    settings: Mutex<Settings>,
    activity: Mutex<Activity>,
}

#[derive(Default)]
struct Activity {
    conversion: Option<(String, CancellationToken)>,
    pending_cancellation: Option<String>,
    inspection: Option<CancellationToken>,
    checkpoints: HashMap<String, PassCheckpoint>,
    closing: bool,
}

impl Application {
    fn snapshot(&self) -> Settings {
        self.settings.lock().expect("settings lock").clone()
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Update {
    job_id: String,
    #[serde(flatten)]
    event: Event,
}

#[tauri::command]
fn get_settings(state: State<'_, Application>) -> Settings {
    state.snapshot()
}

#[tauri::command]
fn save_settings(value: Settings, state: State<'_, Application>) -> Result<Settings, Error> {
    let mut settings = state.settings.lock().expect("settings lock");
    value.save(&state.data_directory.join("settings.json"))?;
    *settings = value;
    Ok(settings.clone())
}

#[tauri::command]
async fn inspect_media(path: PathBuf, state: State<'_, Application>) -> Result<Media, Error> {
    let tools = Tools::locate(&state.snapshot())?;
    let cancel = CancellationToken::new();
    {
        let mut activity = state.activity.lock().expect("activity lock");
        if activity.closing || activity.conversion.is_some() {
            return Err(Error::new("busy", "A conversion is already running.", ""));
        }
        if let Some(previous_inspection) = activity.inspection.replace(cancel.clone()) {
            previous_inspection.cancel();
        }
    }
    finer_engine::inspect(&path, &tools, &cancel).await
}

#[tauri::command]
async fn start_conversion(
    job_id: String,
    queue_id: String,
    path: PathBuf,
    frame_size: Option<FrameSize>,
    app: AppHandle,
    state: State<'_, Application>,
) -> Result<Output, Error> {
    let settings = state.snapshot();
    let tools = Tools::locate(&settings)?;
    let cancel = CancellationToken::new();
    let mut checkpoint = {
        let mut activity = state.activity.lock().expect("activity lock");
        if activity.closing || activity.conversion.is_some() {
            return Err(Error::new("busy", "A conversion is already running.", ""));
        }
        if let Some(inspection) = activity.inspection.take() {
            inspection.cancel();
        }
        if activity.pending_cancellation.take().as_ref() == Some(&job_id) {
            cancel.cancel();
        }
        activity.conversion = Some((job_id.clone(), cancel.clone()));
        activity.checkpoints.remove(&queue_id)
    };
    let conversion = Conversion {
        source: path,
        directory: None,
        frame_size,
        settings,
        log_directory: state.data_directory.join("logs"),
        tools,
    };
    let result = finer_engine::convert_with_retry(
        conversion,
        cancel,
        |event| {
            let update = Update {
                job_id: job_id.clone(),
                event,
            };
            let _ = app.emit_to("main", "conversion-update", update);
        },
        &mut checkpoint,
    )
    .await;
    let closing = {
        let mut activity = state.activity.lock().expect("activity lock");
        activity.conversion = None;
        if !activity.closing {
            if let Some(checkpoint) = checkpoint.take() {
                activity.checkpoints.insert(queue_id, checkpoint);
            }
        }
        activity.closing
    };
    drop(checkpoint);
    if closing {
        app.exit(0);
    }
    result
}

#[tauri::command]
fn discard_checkpoints(queue_ids: Vec<String>, state: State<'_, Application>) {
    let mut activity = state.activity.lock().expect("activity lock");
    for id in queue_ids {
        activity.checkpoints.remove(&id);
    }
}

#[tauri::command]
fn cancel_conversion(job_id: String, state: State<'_, Application>) {
    let mut activity = state.activity.lock().expect("activity lock");
    if let Some((running_job_id, cancel)) = &activity.conversion {
        if *running_job_id == job_id {
            cancel.cancel();
        }
    } else {
        // An immediate click can arrive before the async start command is scheduled.
        activity.pending_cancellation = Some(job_id);
    }
}

#[tauri::command]
fn open_directory(path: PathBuf) -> Result<(), Error> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(Error::new(
            "outputDirectory",
            "The output folder could not be found.",
            path.display(),
        ));
    }
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|error| {
        Error::new(
            "outputDirectory",
            "Could not open the output folder.",
            error,
        )
    })
}

#[tauri::command]
async fn open_result(path: PathBuf, reveal: bool) -> Result<(), Error> {
    if !path.is_absolute() || !path.is_file() {
        return Err(Error::new(
            "missingOutput",
            "The exported video could not be found.",
            path.display(),
        ));
    }
    let result = if reveal {
        tauri_plugin_opener::reveal_item_in_dir(&path)
    } else {
        tauri_plugin_opener::open_path(&path, None::<&str>)
    };
    result.map_err(|error| Error::new("openOutput", "Could not open the exported video.", error))
}

#[tauri::command]
fn confirm_close(app: AppHandle, state: State<'_, Application>) {
    let running = {
        let mut activity = state.activity.lock().expect("activity lock");
        activity.closing = true;
        activity.checkpoints.clear();
        if let Some(inspection) = &activity.inspection {
            inspection.cancel();
        }
        if let Some((_, cancel)) = &activity.conversion {
            cancel.cancel();
        }
        activity.conversion.is_some()
    };
    if !running {
        app.exit(0);
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_directory = app.path().app_local_data_dir()?;
            let settings_path = data_directory.join("settings.json");
            app.manage(Application {
                settings: Mutex::new(Settings::load(&settings_path)),
                activity: Mutex::new(Activity::default()),
                data_directory,
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("close-requested", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            inspect_media,
            start_conversion,
            cancel_conversion,
            discard_checkpoints,
            open_directory,
            open_result,
            confirm_close,
        ])
        .run(tauri::generate_context!())
        .expect("could not start Finer");
}
