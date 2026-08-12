import { invoke } from "@tauri-apps/api/core";

import { isNativeVaultRuntime } from "./vault";

export interface AudioStatus {
  muted: boolean;
  deafened: boolean;
  push_to_talk: boolean;
  ptt_pressed: boolean;
  joined_room: string | null;
}

let previewAudio: AudioStatus = {
  muted: false,
  deafened: false,
  push_to_talk: false,
  ptt_pressed: false,
  joined_room: null,
};

export async function getAudioStatus(): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_status");
  }
  return { ...previewAudio };
}

export async function setAudioMuted(muted: boolean): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_set_muted", { muted });
  }
  if (!previewAudio.deafened) {
    previewAudio = { ...previewAudio, muted };
  }
  return { ...previewAudio };
}

export async function setAudioDeafened(deafened: boolean): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_set_deafened", { deafened });
  }
  previewAudio = {
    ...previewAudio,
    deafened,
    muted: deafened ? true : previewAudio.muted,
  };
  return { ...previewAudio };
}

export async function setAudioPushToTalk(enabled: boolean): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_set_push_to_talk", { enabled });
  }
  previewAudio = {
    ...previewAudio,
    push_to_talk: enabled,
    ptt_pressed: enabled ? previewAudio.ptt_pressed : false,
    muted: enabled ? true : previewAudio.muted,
  };
  return { ...previewAudio };
}

export async function setAudioPttPressed(pressed: boolean): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_set_ptt_pressed", { pressed });
  }
  previewAudio = {
    ...previewAudio,
    ptt_pressed: previewAudio.push_to_talk ? pressed : false,
    muted: previewAudio.push_to_talk ? !pressed : previewAudio.muted,
  };
  return { ...previewAudio };
}

export async function joinAudioRoom(roomId: string): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_join_room", { roomId });
  }
  previewAudio = { ...previewAudio, joined_room: roomId };
  return { ...previewAudio };
}

export async function leaveAudioRoom(): Promise<AudioStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<AudioStatus>("audio_leave_room");
  }
  previewAudio = { ...previewAudio, joined_room: null };
  return { ...previewAudio };
}
