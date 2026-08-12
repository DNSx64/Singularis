import { invoke } from "@tauri-apps/api/core";

import { isNativeVaultRuntime } from "./vault";

export interface VideoStatus {
  camera_enabled: boolean;
  screen_share_enabled: boolean;
}

let previewVideo: VideoStatus = {
  camera_enabled: false,
  screen_share_enabled: false,
};

export async function getVideoStatus(): Promise<VideoStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VideoStatus>("video_status");
  }
  return { ...previewVideo };
}

export async function setCameraEnabled(enabled: boolean): Promise<VideoStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VideoStatus>("video_set_camera_enabled", { enabled });
  }
  previewVideo = {
    camera_enabled: enabled,
    screen_share_enabled: enabled ? false : previewVideo.screen_share_enabled,
  };
  return { ...previewVideo };
}

export async function setScreenShareEnabled(enabled: boolean): Promise<VideoStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VideoStatus>("video_set_screen_share_enabled", { enabled });
  }
  previewVideo = {
    camera_enabled: enabled ? false : previewVideo.camera_enabled,
    screen_share_enabled: enabled,
  };
  return { ...previewVideo };
}
