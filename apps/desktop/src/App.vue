<script setup lang="ts">
import {
  Archive,
  BellOff,
  ChevronDown,
  CircleHelp,
  Database,
  FileText,
  Hash,
  Headphones,
  Inbox,
  LockKeyhole,
  Menu,
  MessageSquare,
  Mic,
  MonitorSmartphone,
  MoreHorizontal,
  Paperclip,
  Plus,
  RefreshCw,
  Search,
  Send,
  Server,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Users,
  Video,
  Vault,
  Volume2,
  X,
} from "@lucide/vue";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";

import singularisLogo from "../../../SingularisLogo_V1.png";
import {
  getAudioStatus,
  joinAudioRoom,
  leaveAudioRoom,
  setAudioPttPressed,
  setAudioDeafened,
  setAudioMuted,
  setAudioPushToTalk,
  type AudioStatus,
} from "./audio";
import {
  getVideoStatus,
  setCameraEnabled,
  setScreenShareEnabled,
  type VideoStatus,
} from "./video";
import { fetchHealth, type HealthResponse } from "./api";
import {
  getVaultStatus,
  getOutboxStatus,
  initializeVault,
  isNativeVaultRuntime,
  listVaultMessages,
  lockVault,
  queueVaultMessage,
  searchVaultMessages,
  retryOutbox,
  unlockVault,
  vaultErrorMessage,
  type VaultMessage,
  type OutboxWorkerStatus,
  type VaultStatus,
} from "./vault";

type ConnectionState = "checking" | "online" | "offline";

interface Channel {
  id: string;
  name: string;
  unread?: number;
  kind: "text" | "voice";
}

interface ChatMessage {
  id: string;
  author: string;
  initials: string;
  time: string;
  body: string;
  accent: string;
  vaultBacked?: boolean;
  system?: boolean;
}

const textChannels: Channel[] = [
  { id: "briefing", name: "briefing", kind: "text" },
  { id: "entwicklung", name: "entwicklung", unread: 3, kind: "text" },
  { id: "security", name: "security-review", kind: "text" },
  { id: "offtopic", name: "offtopic", kind: "text" },
];

const voiceChannels: Channel[] = [
  { id: "lounge", name: "Lounge", kind: "voice" },
  { id: "pairing", name: "Pairing Room", kind: "voice" },
];

const messages = reactive<Record<string, ChatMessage[]>>({
  briefing: [
    {
      id: "b1",
      author: "System",
      initials: "S",
      time: "09:00",
      body: "Dieser Kanal hält Serverkopien sieben Tage. Dein lokales Archiv folgt deiner eigenen Aufbewahrungsregel.",
      accent: "#25dfd3",
      system: true,
    },
    {
      id: "b2",
      author: "Mara",
      initials: "MA",
      time: "09:18",
      body: "Ich habe die ersten Ablaufgrenzen für den Event-Spool eingetragen. Die Serversequenz bleibt nur eine Anzeigehilfe, kein Vertrauensanker.",
      accent: "#f0a85a",
    },
    {
      id: "b3",
      author: "Jonas",
      initials: "JO",
      time: "09:24",
      body: "Passt. Als Nächstes sollten wir den Gerätewechsel einmal vollständig durchspielen, bevor wir weitere Plattformen anbinden.",
      accent: "#8cb7ff",
    },
    {
      id: "b4",
      author: "Lea",
      initials: "LE",
      time: "09:31",
      body: "Ich übernehme den Recovery-Prototyp. Der Export bleibt unabhängig vom Server und enthält keine stillen Cloud-Abhängigkeiten.",
      accent: "#d5a4ff",
    },
  ],
  entwicklung: [
    {
      id: "e1",
      author: "Noah",
      initials: "NO",
      time: "Gestern",
      body: "Der neue Protocol-Crate validiert die TTL schon beim Deserialisieren. Werte über sieben Tagen erreichen den Store damit nicht.",
      accent: "#7fd395",
    },
    {
      id: "e2",
      author: "Mara",
      initials: "MA",
      time: "08:52",
      body: "Gut. Bitte auch den Fall testen, dass dieselbe Event-ID mit verändertem Ciphertext erneut gesendet wird.",
      accent: "#f0a85a",
    },
  ],
  security: [
    {
      id: "s1",
      author: "Security Bot",
      initials: "SB",
      time: "08:10",
      body: "Keine offenen Identitätswarnungen. Zwei Geräte sind manuell verifiziert.",
      accent: "#25dfd3",
      system: true,
    },
  ],
  offtopic: [],
});

const activeChannelId = ref("briefing");
const composerText = ref("");
const searchQuery = ref("");
const searchOpen = ref(false);
const detailsOpen = ref(true);
const navigationOpen = ref(false);
const connectionState = ref<ConnectionState>("checking");
const health = ref<HealthResponse | null>(null);
const nativeVaultRuntime = isNativeVaultRuntime();
const vaultStatus = ref<VaultStatus | null>(null);
const vaultLoading = ref(nativeVaultRuntime);
const vaultBusy = ref(false);
const vaultError = ref("");
const vaultPassphrase = ref("");
const vaultConfirmation = ref("");
const previewLocked = ref(false);
const composerBusy = ref(false);
const composerError = ref("");
const outboxStatus = ref<OutboxWorkerStatus | null>(null);
const outboxBusy = ref(false);
const audioStatus = ref<AudioStatus | null>(null);
const audioError = ref("");
const micCaptureState = ref<"idle" | "active" | "unsupported" | "denied">("idle");
const micLevel = ref(0);
const videoStatus = ref<VideoStatus | null>(null);
const videoError = ref("");
const videoCaptureState = ref<"idle" | "active" | "unsupported" | "denied">("idle");
const videoPreview = ref<HTMLVideoElement | null>(null);
const vaultSearchMessageIds = ref<Set<string> | null>(null);
let vaultSearchRevision = 0;
let outboxStatusTimer: number | undefined;
let micStream: MediaStream | null = null;
let micContext: AudioContext | null = null;
let micAnalyser: AnalyserNode | null = null;
let micFrame: number | undefined;
let pttHolding = false;
let videoStream: MediaStream | null = null;

const activeChannel = computed(
  () => textChannels.find((channel) => channel.id === activeChannelId.value) ?? textChannels[0]!,
);

const activeMessages = computed(() => {
  const channelMessages = messages[activeChannelId.value] ?? [];
  const query = searchQuery.value.trim().toLocaleLowerCase("de");

  if (!query) {
    return channelMessages;
  }

  return channelMessages.filter((message) => {
    if (message.vaultBacked && nativeVaultRuntime) {
      return vaultSearchMessageIds.value?.has(message.id) ?? false;
    }
    return (
      message.author.toLocaleLowerCase("de").includes(query) ||
      message.body.toLocaleLowerCase("de").includes(query)
    );
  });
});

const vaultIsUnlocked = computed(
  () => (!nativeVaultRuntime && !previewLocked.value) || vaultStatus.value?.state === "unlocked",
);

const vaultGateVisible = computed(
  () => previewLocked.value || (nativeVaultRuntime && vaultStatus.value?.state !== "unlocked"),
);

const vaultIsUninitialized = computed(() => vaultStatus.value?.state === "uninitialized");

const vaultGateTitle = computed(() => {
  if (vaultLoading.value) {
    return "Vault wird geprüft";
  }
  if (vaultIsUninitialized.value) {
    return "Lokalen Vault einrichten";
  }
  return "Lokaler Vault gesperrt";
});

const vaultGateCopy = computed(() => {
  if (vaultIsUninitialized.value) {
    return "Die Passphrase schützt den zufälligen Schlüssel deines lokalen SQLCipher-Archivs.";
  }
  return "Entsperre den lokalen Vault, um Nachrichten und Suche wieder freizugeben.";
});

const serverTtlLabel = computed(() => {
  if (!health.value) {
    return "7 Tage";
  }

  const days = health.value.max_server_ttl_seconds / 86_400;
  return Number.isInteger(days) ? `${days} Tage` : `${health.value.max_server_ttl_seconds}s`;
});

const outboxStatusLabel = computed(() => {
  if (!nativeVaultRuntime) {
    return "Lokale Vorschau · keine Relay-Übertragung";
  }
  if (!vaultIsUnlocked.value || outboxStatus.value?.state === "paused") {
    return "Verschlüsselte Outbox pausiert";
  }
  if (!outboxStatus.value) {
    return "Outboxstatus wird geprüft";
  }
  if (outboxStatus.value.state === "sending") {
    return `Outbox sendet · ${outboxStatus.value.pending} ausstehend`;
  }
  if (outboxStatus.value.state === "deferred") {
    return `Relay wartet · ${outboxStatus.value.pending} ausstehend`;
  }
  return outboxStatus.value.pending > 0
    ? `${outboxStatus.value.pending} verschlüsselt ausstehend`
    : "Verschlüsselte Outbox synchron";
});

const joinedVoiceRoom = computed(() => audioStatus.value?.joined_room ?? null);

const voiceStatusLabel = computed(() => {
  if (!audioStatus.value?.joined_room) {
    return "Nicht verbunden";
  }
  if (audioStatus.value.deafened) {
    return `${audioStatus.value.joined_room} · Audio aus`;
  }
  if (audioStatus.value.muted) {
    return `${audioStatus.value.joined_room} · Mikro stumm`;
  }
  return `${audioStatus.value.joined_room} · Live`;
});

const canMonitorMic = computed(() =>
  Boolean(
    audioStatus.value?.joined_room &&
      !audioStatus.value.muted &&
      !audioStatus.value.deafened,
  ),
);

const micLevelPercent = computed(() => `${Math.round(micLevel.value * 100)}%`);

const micButtonLabel = computed(() => {
  if (!audioStatus.value) {
    return "Mikrofon";
  }
  return audioStatus.value.muted ? "Mikrofon aktivieren" : "Mikrofon stummschalten";
});

const headphoneButtonLabel = computed(() => {
  if (!audioStatus.value) {
    return "Audio";
  }
  return audioStatus.value.deafened ? "Audio einschalten" : "Audio ausschalten";
});

const pttButtonLabel = computed(() => {
  if (!audioStatus.value) {
    return "Push-to-talk aktivieren";
  }
  return audioStatus.value.push_to_talk
    ? "Push-to-talk deaktivieren"
    : "Push-to-talk aktivieren";
});

const canUseVideo = computed(() => Boolean(audioStatus.value?.joined_room));

const videoButtonLabel = computed(() => {
  if (!videoStatus.value) {
    return "Kamera einschalten";
  }
  return videoStatus.value.camera_enabled ? "Kamera ausschalten" : "Kamera einschalten";
});

const screenShareButtonLabel = computed(() => {
  if (!videoStatus.value) {
    return "Bildschirm teilen";
  }
  return videoStatus.value.screen_share_enabled
    ? "Bildschirmfreigabe stoppen"
    : "Bildschirm teilen";
});

const videoStatusLabel = computed(() => {
  if (!canUseVideo.value) {
    return "Kein Sprachraum aktiv";
  }
  if (videoStatus.value?.screen_share_enabled) {
    return "Bildschirmfreigabe aktiv";
  }
  if (videoStatus.value?.camera_enabled) {
    return "Kamera aktiv";
  }
  return "Video aus";
});

async function refreshHealth(): Promise<void> {
  connectionState.value = "checking";
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), 2_500);

  try {
    health.value = await fetchHealth(controller.signal);
    connectionState.value = "online";
  } catch {
    health.value = null;
    connectionState.value = "offline";
  } finally {
    window.clearTimeout(timeout);
  }
}

function selectChannel(channelId: string): void {
  activeChannelId.value = channelId;
  navigationOpen.value = false;
  searchQuery.value = "";
}

async function submitMessage(): Promise<void> {
  const body = composerText.value.trim();
  if (!body || !vaultIsUnlocked.value || composerBusy.value) {
    return;
  }

  composerBusy.value = true;
  composerError.value = "";
  const message = {
    id: crypto.randomUUID(),
    channel_id: activeChannelId.value,
    body,
    created_at_ms: Date.now(),
  };

  try {
    const stored = await queueVaultMessage(message);
    const channelMessages = messages[activeChannelId.value] ?? [];
    channelMessages.push(toChatMessage(stored));
    messages[activeChannelId.value] = channelMessages;
    composerText.value = "";
    vaultStatus.value = await getVaultStatus();
    await refreshOutboxStatus();
  } catch (error) {
    composerError.value = vaultErrorMessage(error);
  } finally {
    composerBusy.value = false;
  }
}

async function refreshVaultState(): Promise<void> {
  vaultLoading.value = true;
  vaultError.value = "";
  try {
    vaultStatus.value = await getVaultStatus();
    if (vaultStatus.value.state === "unlocked") {
      await loadVaultMessages();
    }
    await refreshOutboxStatus();
  } catch (error) {
    vaultStatus.value = null;
    vaultError.value = vaultErrorMessage(error);
  } finally {
    vaultLoading.value = false;
  }
}

async function submitVaultGate(): Promise<void> {
  if (!nativeVaultRuntime) {
    await unlockPreview();
    return;
  }
  if (!vaultStatus.value || vaultBusy.value) {
    return;
  }
  if (vaultIsUninitialized.value && vaultPassphrase.value !== vaultConfirmation.value) {
    vaultError.value = "Die Passphrasen stimmen nicht überein.";
    return;
  }

  vaultBusy.value = true;
  vaultError.value = "";
  try {
    vaultStatus.value = vaultIsUninitialized.value
      ? await initializeVault(vaultPassphrase.value)
      : await unlockVault(vaultPassphrase.value);
    await loadVaultMessages();
    await refreshOutboxStatus();
  } catch (error) {
    vaultError.value = vaultErrorMessage(error);
  } finally {
    vaultPassphrase.value = "";
    vaultConfirmation.value = "";
    vaultBusy.value = false;
  }
}

async function quickLock(): Promise<void> {
  if (vaultBusy.value) {
    return;
  }
  vaultBusy.value = true;
  vaultError.value = "";
  try {
    if (videoStatus.value?.camera_enabled) {
      videoStatus.value = await setCameraEnabled(false);
    }
    if (videoStatus.value?.screen_share_enabled) {
      videoStatus.value = await setScreenShareEnabled(false);
    }
    clearVideoCapture();
    vaultStatus.value = await lockVault();
    previewLocked.value = !nativeVaultRuntime;
    stopMicMonitoring();
    clearVaultBackedState();
    await refreshOutboxStatus();
  } catch (error) {
    vaultError.value = vaultErrorMessage(error);
  } finally {
    vaultBusy.value = false;
  }
}

async function unlockPreview(): Promise<void> {
  vaultStatus.value = await unlockVault("");
  previewLocked.value = false;
  await loadVaultMessages();
  await refreshOutboxStatus();
}

async function refreshOutboxStatus(): Promise<void> {
  try {
    outboxStatus.value = await getOutboxStatus();
  } catch (error) {
    if (nativeVaultRuntime) {
      composerError.value = vaultErrorMessage(error);
    }
  }
}

async function retryPendingOutbox(): Promise<void> {
  if (!nativeVaultRuntime || !vaultIsUnlocked.value || outboxBusy.value) {
    return;
  }
  outboxBusy.value = true;
  composerError.value = "";
  try {
    outboxStatus.value = await retryOutbox();
  } catch (error) {
    composerError.value = vaultErrorMessage(error);
  } finally {
    outboxBusy.value = false;
  }
}

async function refreshAudioState(): Promise<void> {
  try {
    audioStatus.value = await getAudioStatus();
    if (canMonitorMic.value) {
      await ensureMicMonitoring();
    } else {
      stopMicMonitoring();
    }
  } catch (error) {
    audioError.value = vaultErrorMessage(error);
  }
}

async function refreshVideoState(): Promise<void> {
  try {
    videoStatus.value = await getVideoStatus();
    await syncVideoCapture();
  } catch (error) {
    videoError.value = vaultErrorMessage(error);
  }
}

async function toggleMute(): Promise<void> {
  if (!audioStatus.value) {
    await refreshAudioState();
    return;
  }
  audioError.value = "";
  try {
    audioStatus.value = await setAudioMuted(!audioStatus.value.muted);
    if (canMonitorMic.value) {
      await ensureMicMonitoring();
    } else {
      stopMicMonitoring();
    }
  } catch (error) {
    audioError.value = vaultErrorMessage(error);
  }
}

async function toggleDeafened(): Promise<void> {
  if (!audioStatus.value) {
    await refreshAudioState();
    return;
  }
  audioError.value = "";
  try {
    audioStatus.value = await setAudioDeafened(!audioStatus.value.deafened);
    if (canMonitorMic.value) {
      await ensureMicMonitoring();
    } else {
      stopMicMonitoring();
    }
  } catch (error) {
    audioError.value = vaultErrorMessage(error);
  }
}

async function toggleVoiceRoom(roomId: string): Promise<void> {
  audioError.value = "";
  try {
    audioStatus.value =
      joinedVoiceRoom.value === roomId ? await leaveAudioRoom() : await joinAudioRoom(roomId);
    if (!audioStatus.value.joined_room && videoStatus.value) {
      videoStatus.value = await setCameraEnabled(false);
      videoStatus.value = await setScreenShareEnabled(false);
    }
    if (canMonitorMic.value) {
      await ensureMicMonitoring();
    } else {
      stopMicMonitoring();
    }
    await syncVideoCapture();
  } catch (error) {
    audioError.value = vaultErrorMessage(error);
  }
}

async function toggleCamera(): Promise<void> {
  if (!canUseVideo.value) {
    videoError.value = "Verbinde zuerst einen Sprachraum.";
    return;
  }
  videoError.value = "";
  try {
    videoStatus.value = await setCameraEnabled(!videoStatus.value?.camera_enabled);
    await syncVideoCapture();
  } catch (error) {
    videoError.value = vaultErrorMessage(error);
  }
}

async function toggleScreenShare(): Promise<void> {
  if (!canUseVideo.value) {
    videoError.value = "Verbinde zuerst einen Sprachraum.";
    return;
  }
  videoError.value = "";
  try {
    videoStatus.value = await setScreenShareEnabled(!videoStatus.value?.screen_share_enabled);
    await syncVideoCapture();
  } catch (error) {
    videoError.value = vaultErrorMessage(error);
  }
}

async function togglePushToTalk(): Promise<void> {
  if (!audioStatus.value) {
    await refreshAudioState();
    return;
  }
  audioError.value = "";
  try {
    audioStatus.value = await setAudioPushToTalk(!audioStatus.value.push_to_talk);
    if (!audioStatus.value.push_to_talk && pttHolding) {
      pttHolding = false;
    }
    if (canMonitorMic.value) {
      await ensureMicMonitoring();
    } else {
      stopMicMonitoring();
    }
  } catch (error) {
    audioError.value = vaultErrorMessage(error);
  }
}

async function setPttPressed(pressed: boolean): Promise<void> {
  if (!audioStatus.value?.push_to_talk || pttHolding === pressed) {
    return;
  }
  pttHolding = pressed;
  try {
    audioStatus.value = await setAudioPttPressed(pressed);
    if (canMonitorMic.value) {
      await ensureMicMonitoring();
    } else {
      stopMicMonitoring();
    }
  } catch (error) {
    audioError.value = vaultErrorMessage(error);
  }
}

async function ensureMicMonitoring(): Promise<void> {
  if (micCaptureState.value === "active") {
    return;
  }
  if (
    typeof navigator === "undefined" ||
    !navigator.mediaDevices ||
    typeof navigator.mediaDevices.getUserMedia !== "function"
  ) {
    micCaptureState.value = "unsupported";
    micLevel.value = 0;
    return;
  }

  try {
    micStream = await navigator.mediaDevices.getUserMedia({ audio: true });
    micContext = new AudioContext();
    const source = micContext.createMediaStreamSource(micStream);
    micAnalyser = micContext.createAnalyser();
    micAnalyser.fftSize = 256;
    micAnalyser.smoothingTimeConstant = 0.78;
    source.connect(micAnalyser);
    micCaptureState.value = "active";
    loopMicLevel();
  } catch {
    micCaptureState.value = "denied";
    micLevel.value = 0;
    audioError.value = "Mikrofonzugriff wurde blockiert.";
  }
}

function stopMicMonitoring(): void {
  if (micFrame !== undefined) {
    window.cancelAnimationFrame(micFrame);
    micFrame = undefined;
  }
  micAnalyser = null;
  if (micStream) {
    for (const track of micStream.getTracks()) {
      track.stop();
    }
  }
  micStream = null;
  if (micContext) {
    void micContext.close();
  }
  micContext = null;
  if (micCaptureState.value !== "unsupported") {
    micCaptureState.value = "idle";
  }
  micLevel.value = 0;
}

async function syncVideoCapture(): Promise<void> {
  if (!canUseVideo.value || !videoStatus.value) {
    clearVideoCapture();
    return;
  }
  if (videoStatus.value.screen_share_enabled) {
    await ensureScreenShareCapture();
    return;
  }
  if (videoStatus.value.camera_enabled) {
    await ensureCameraCapture();
    return;
  }
  clearVideoCapture();
}

async function ensureCameraCapture(): Promise<void> {
  if (
    typeof navigator === "undefined" ||
    !navigator.mediaDevices ||
    typeof navigator.mediaDevices.getUserMedia !== "function"
  ) {
    videoCaptureState.value = "unsupported";
    clearVideoCapture();
    return;
  }
  clearVideoCapture();
  try {
    videoStream = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
    attachVideoPreview(videoStream);
    videoCaptureState.value = "active";
  } catch {
    videoCaptureState.value = "denied";
    videoError.value = "Kamerazugriff wurde blockiert.";
    if (videoStatus.value?.camera_enabled) {
      videoStatus.value = await setCameraEnabled(false);
    }
  }
}

async function ensureScreenShareCapture(): Promise<void> {
  if (
    typeof navigator === "undefined" ||
    !navigator.mediaDevices ||
    typeof navigator.mediaDevices.getDisplayMedia !== "function"
  ) {
    videoCaptureState.value = "unsupported";
    clearVideoCapture();
    return;
  }
  clearVideoCapture();
  try {
    videoStream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
    const [track] = videoStream.getVideoTracks();
    if (track) {
      track.onended = () => {
        void setScreenShareEnabled(false).then((status) => {
          videoStatus.value = status;
          clearVideoCapture();
        });
      };
    }
    attachVideoPreview(videoStream);
    videoCaptureState.value = "active";
  } catch {
    videoCaptureState.value = "denied";
    videoError.value = "Bildschirmfreigabe wurde blockiert.";
    if (videoStatus.value?.screen_share_enabled) {
      videoStatus.value = await setScreenShareEnabled(false);
    }
  }
}

function attachVideoPreview(stream: MediaStream): void {
  if (!videoPreview.value) {
    return;
  }
  videoPreview.value.srcObject = stream;
  void videoPreview.value.play().catch(() => {});
}

function clearVideoCapture(): void {
  if (videoStream) {
    for (const track of videoStream.getTracks()) {
      track.onended = null;
      track.stop();
    }
  }
  videoStream = null;
  if (videoPreview.value) {
    videoPreview.value.srcObject = null;
  }
  if (videoCaptureState.value !== "unsupported") {
    videoCaptureState.value = "idle";
  }
}

function loopMicLevel(): void {
  if (!micAnalyser) {
    return;
  }
  const samples = new Float32Array(micAnalyser.fftSize);
  const render = () => {
    if (!micAnalyser) {
      return;
    }
    micAnalyser.getFloatTimeDomainData(samples);
    let sumSquares = 0;
    for (const sample of samples) {
      sumSquares += sample * sample;
    }
    const rms = Math.sqrt(sumSquares / samples.length);
    micLevel.value = Math.min(1, rms * 3.2);
    micFrame = window.requestAnimationFrame(render);
  };
  micFrame = window.requestAnimationFrame(render);
}

async function loadVaultMessages(): Promise<void> {
  const storedByChannel = await Promise.all(
    textChannels.map(async (channel) => [channel.id, await listVaultMessages(channel.id)] as const),
  );
  for (const [channelId, storedMessages] of storedByChannel) {
    const previewMessages = (messages[channelId] ?? []).filter((message) => !message.vaultBacked);
    messages[channelId] = [...previewMessages, ...storedMessages.map(toChatMessage)];
  }
  vaultStatus.value = await getVaultStatus();
}

function clearVaultBackedState(): void {
  for (const channel of textChannels) {
    messages[channel.id] = (messages[channel.id] ?? []).filter((message) => !message.vaultBacked);
  }
  composerText.value = "";
  composerError.value = "";
  searchQuery.value = "";
  searchOpen.value = false;
  vaultSearchMessageIds.value = null;
}

function toChatMessage(message: VaultMessage): ChatMessage {
  return {
    id: message.id,
    author: "Du",
    initials: "DU",
    time: new Intl.DateTimeFormat("de-DE", {
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(message.created_at_ms)),
    body: message.body,
    accent: "#25dfd3",
    vaultBacked: true,
  };
}

function toggleSearch(): void {
  searchOpen.value = !searchOpen.value;
  if (!searchOpen.value) {
    searchQuery.value = "";
  }
}

function handleKeydown(event: KeyboardEvent): void {
  if (event.ctrlKey && event.shiftKey && event.key.toLocaleLowerCase() === "l") {
    event.preventDefault();
    void quickLock();
  }

  if (event.key === "Escape") {
    searchOpen.value = false;
    navigationOpen.value = false;
    searchQuery.value = "";
  }

  if (
    audioStatus.value?.push_to_talk &&
    event.code === "KeyV" &&
    !event.repeat &&
    !isEditableTarget(event.target)
  ) {
    event.preventDefault();
    void setPttPressed(true);
  }
}

function handleKeyup(event: KeyboardEvent): void {
  if (audioStatus.value?.push_to_talk && event.code === "KeyV") {
    void setPttPressed(false);
  }
}

function handleWindowBlur(): void {
  if (pttHolding) {
    void setPttPressed(false);
  }
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  if (target.isContentEditable) {
    return true;
  }
  return target.tagName === "INPUT" || target.tagName === "TEXTAREA";
}

onMounted(() => {
  void refreshHealth();
  void refreshVaultState();
  void refreshOutboxStatus();
  void refreshAudioState();
  void refreshVideoState();
  if (nativeVaultRuntime) {
    outboxStatusTimer = window.setInterval(() => void refreshOutboxStatus(), 3_000);
  }
  window.addEventListener("keydown", handleKeydown);
  window.addEventListener("keyup", handleKeyup);
  window.addEventListener("blur", handleWindowBlur);
});

onUnmounted(() => {
  stopMicMonitoring();
  clearVideoCapture();
  window.removeEventListener("keydown", handleKeydown);
  window.removeEventListener("keyup", handleKeyup);
  window.removeEventListener("blur", handleWindowBlur);
  if (outboxStatusTimer !== undefined) {
    window.clearInterval(outboxStatusTimer);
  }
});

watch(videoPreview, () => {
  if (videoPreview.value && videoStream) {
    attachVideoPreview(videoStream);
  }
});

watch([searchQuery, activeChannelId], ([query, channelId]) => {
  const normalizedQuery = query.trim();
  const revision = ++vaultSearchRevision;
  vaultSearchMessageIds.value = null;
  if (!nativeVaultRuntime || !normalizedQuery || !vaultIsUnlocked.value) {
    return;
  }

  void searchVaultMessages(channelId, normalizedQuery)
    .then((results) => {
      if (revision === vaultSearchRevision) {
        vaultSearchMessageIds.value = new Set(results.map((message) => message.id));
      }
    })
    .catch((error: unknown) => {
      if (revision === vaultSearchRevision) {
        vaultError.value = vaultErrorMessage(error);
        vaultSearchMessageIds.value = new Set();
      }
    });
});
</script>

<template>
  <div class="app-shell" :class="{ 'details-collapsed': !detailsOpen }">
    <nav class="app-rail" aria-label="Hauptnavigation">
      <button class="rail-brand" type="button" aria-label="Singularis" title="Singularis">
        S
      </button>

      <div class="rail-rule" />

      <button class="rail-button active" type="button" aria-label="Nachrichten" title="Nachrichten">
        <MessageSquare :size="20" />
      </button>
      <button class="rail-button" type="button" aria-label="Lokales Archiv" title="Lokales Archiv">
        <Archive :size="20" />
      </button>
      <button class="rail-button" type="button" aria-label="Sicherheitszentrum" title="Sicherheitszentrum">
        <ShieldCheck :size="20" />
      </button>

      <div class="rail-spacer" />

      <button class="rail-button" type="button" aria-label="Hilfe" title="Hilfe">
        <CircleHelp :size="20" />
      </button>
      <button class="rail-button" type="button" aria-label="Einstellungen" title="Einstellungen">
        <Settings :size="20" />
      </button>
      <button
        class="rail-button lock-button"
        type="button"
        aria-label="Oberfläche sperren"
        title="Sperren"
        @click="quickLock"
      >
        <LockKeyhole :size="20" />
      </button>
    </nav>

    <aside class="channel-sidebar" :class="{ open: navigationOpen }">
      <header class="brand-header">
        <img :src="singularisLogo" alt="Singularis" class="wordmark" />
        <span class="build-label">LOCAL</span>
      </header>

      <button class="community-switcher" type="button">
        <span class="community-mark">NX</span>
        <span class="community-copy">
          <strong>Nexus Lab</strong>
          <small>4 Mitglieder online</small>
        </span>
        <ChevronDown :size="16" />
      </button>

      <div class="stream-status">
        <span class="pulse-dot" :class="connectionState" />
        <span>
          <small>SERVER-STREAM</small>
          <strong>{{ serverTtlLabel }} TTL</strong>
        </span>
        <button
          type="button"
          aria-label="Serverstatus aktualisieren"
          title="Serverstatus aktualisieren"
          @click="refreshHealth"
        >
          <Server :size="16" />
        </button>
      </div>

      <div class="channel-scroll">
        <section class="channel-group">
          <header>
            <span>Textkanäle</span>
            <button type="button" aria-label="Textkanal hinzufügen" title="Textkanal hinzufügen">
              <Plus :size="15" />
            </button>
          </header>
          <button
            v-for="channel in textChannels"
            :key="channel.id"
            class="channel-button"
            :class="{ active: activeChannelId === channel.id }"
            type="button"
            @click="selectChannel(channel.id)"
          >
            <Hash :size="17" />
            <span>{{ channel.name }}</span>
            <span v-if="channel.unread" class="unread-count">{{ channel.unread }}</span>
          </button>
        </section>

        <section class="channel-group voice-group">
          <header>
            <span>Sprachräume</span>
            <button type="button" aria-label="Sprachraum hinzufügen" title="Sprachraum hinzufügen">
              <Plus :size="15" />
            </button>
          </header>
          <button
            v-for="channel in voiceChannels"
            :key="channel.id"
            class="channel-button"
            :class="{ active: joinedVoiceRoom === channel.id }"
            type="button"
            @click="toggleVoiceRoom(channel.id)"
          >
            <Volume2 :size="17" />
            <span>{{ channel.name }}</span>
            <span v-if="joinedVoiceRoom === channel.id" class="voice-live">LIVE</span>
          </button>
        </section>
      </div>

      <footer class="account-bar">
        <span class="avatar self">V</span>
        <span class="account-copy">
          <strong>void</strong>
          <small>
            <span class="online-dot" /> Gerät verifiziert · {{ voiceStatusLabel }}
            <template v-if="audioStatus?.push_to_talk"> · PTT (Taste V)</template>
          </small>
        </span>
        <span class="mic-meter" :class="micCaptureState" :aria-label="`Mikrofonpegel ${micLevelPercent}`">
          <span :style="{ width: micLevelPercent }" />
        </span>
        <button
          type="button"
          class="ptt-button"
          :class="{ active: audioStatus?.push_to_talk, holding: audioStatus?.ptt_pressed }"
          :aria-label="pttButtonLabel"
          :title="pttButtonLabel"
          @click="togglePushToTalk"
        >
          PTT
        </button>
        <button
          type="button"
          :class="{ active: audioStatus && !audioStatus.muted }"
          :aria-label="micButtonLabel"
          :title="micButtonLabel"
          @click="toggleMute"
        >
          <Mic :size="16" />
        </button>
        <button
          type="button"
          :class="{ active: audioStatus && !audioStatus.deafened }"
          :aria-label="headphoneButtonLabel"
          :title="headphoneButtonLabel"
          @click="toggleDeafened"
        >
          <Headphones :size="16" />
        </button>
      </footer>
      <span v-if="audioError" class="account-audio-error" role="alert">{{ audioError }}</span>
    </aside>

    <main class="chat-panel">
      <header class="chat-header">
        <button
          class="mobile-menu"
          type="button"
          aria-label="Kanäle öffnen"
          title="Kanäle öffnen"
          @click="navigationOpen = !navigationOpen"
        >
          <Menu :size="20" />
        </button>
        <Hash :size="19" class="channel-hash" />
        <div class="channel-heading">
          <strong>{{ activeChannel.name }}</strong>
          <span>Lokale Arbeitsgruppe</span>
        </div>

        <div class="header-spacer" />

        <label v-if="searchOpen" class="search-field">
          <Search :size="15" />
          <input v-model="searchQuery" type="search" placeholder="Lokal suchen" autofocus />
          <button type="button" aria-label="Suche schließen" title="Suche schließen" @click="toggleSearch">
            <X :size="15" />
          </button>
        </label>
        <button v-else class="header-action" type="button" aria-label="Lokal suchen" title="Lokal suchen" @click="toggleSearch">
          <Search :size="18" />
        </button>
        <button class="header-action" type="button" aria-label="Benachrichtigungen" title="Benachrichtigungen">
          <BellOff :size="18" />
        </button>
        <button
          class="header-action"
          :class="{ active: detailsOpen }"
          type="button"
          aria-label="Details einblenden"
          title="Details einblenden"
          @click="detailsOpen = !detailsOpen"
        >
          <SlidersHorizontal :size="18" />
        </button>
      </header>

      <div class="security-strip">
        <span><ShieldCheck :size="14" /> {{ nativeVaultRuntime ? "MLS-Outbox aktiv" : "E2EE nur im Desktop" }}</span>
        <span><Vault :size="14" /> {{ nativeVaultRuntime ? "SQLCipher-Vault" : "Lokale Vorschau" }}</span>
        <span class="connection-copy" :class="connectionState">
          <span class="pulse-dot" :class="connectionState" />
          {{ connectionState === "online" ? "Relay erreichbar" : connectionState === "checking" ? "Relay wird geprüft" : "Relay offline" }}
        </span>
      </div>

      <section class="message-scroll" :aria-label="`Nachrichten in ${activeChannel.name}`">
        <div class="channel-intro">
          <div class="intro-icon"><Hash :size="24" /></div>
          <h1>{{ activeChannel.name }}</h1>
          <p>Beginn deines lokalen Verlaufs in diesem Kanal.</p>
        </div>

        <div v-if="activeMessages.length === 0" class="empty-state">
          <Inbox :size="28" />
          <strong>Keine lokalen Treffer</strong>
          <span>Der Server führt keinen Suchindex.</span>
        </div>

        <article
          v-for="message in activeMessages"
          :key="message.id"
          class="message-row"
          :class="{ system: message.system }"
        >
          <span class="message-avatar" :style="{ '--avatar-accent': message.accent }">
            {{ message.initials }}
          </span>
          <div class="message-content">
            <header>
              <strong>{{ message.author }}</strong>
              <time>{{ message.time }}</time>
              <span v-if="message.vaultBacked" class="local-badge">
                <ShieldCheck v-if="nativeVaultRuntime" :size="12" />
                <Database v-else :size="12" />
                {{ nativeVaultRuntime ? "MLS-OUTBOX" : "NUR LOKAL" }}
              </span>
            </header>
            <p>{{ message.body }}</p>
          </div>
          <button class="message-menu" type="button" aria-label="Nachrichtenoptionen" title="Nachrichtenoptionen">
            <MoreHorizontal :size="17" />
          </button>
        </article>
      </section>

      <footer class="composer-wrap">
        <div class="outbox-notice" :class="{ error: composerError }">
          <Sparkles :size="14" />
          <span>{{ composerError || outboxStatusLabel }}</span>
          <button
            v-if="nativeVaultRuntime && vaultIsUnlocked && (outboxStatus?.pending || outboxStatus?.state === 'deferred')"
            type="button"
            :disabled="outboxBusy"
            aria-label="Outbox erneut senden"
            title="Outbox erneut senden"
            @click="retryPendingOutbox"
          >
            <RefreshCw :size="13" />
          </button>
        </div>
        <form class="composer" @submit.prevent="submitMessage">
          <button type="button" aria-label="Datei anhängen" title="Datei anhängen">
            <Paperclip :size="19" />
          </button>
          <textarea
            v-model="composerText"
            :placeholder="`Nachricht an #${activeChannel.name}`"
            :disabled="!vaultIsUnlocked || composerBusy"
            rows="1"
            @keydown.enter.exact.prevent="submitMessage"
          />
          <button
            class="send-button"
            type="submit"
            :disabled="!composerText.trim() || !vaultIsUnlocked || composerBusy"
            :aria-label="nativeVaultRuntime ? 'Verschlüsselt senden' : 'Lokal speichern'"
            :title="nativeVaultRuntime ? 'Verschlüsselt senden' : 'Lokal speichern'"
          >
            <Send :size="18" />
          </button>
        </form>
      </footer>
    </main>

    <aside class="details-panel">
      <header class="details-header">
        <span>Kanalstatus</span>
        <button type="button" aria-label="Details schließen" title="Details schließen" @click="detailsOpen = false">
          <X :size="17" />
        </button>
      </header>

      <section class="detail-section ttl-section">
        <header>
          <span>Serverkopie</span>
          <strong>{{ serverTtlLabel }}</strong>
        </header>
        <div class="ttl-track"><span /></div>
        <div class="detail-pair">
          <span>Frühester Ablauf</span>
          <strong>Heute, 16:42</strong>
        </div>
        <div class="detail-pair">
          <span>Speichermodus</span>
          <strong>Ciphertext</strong>
        </div>
      </section>

      <section class="detail-section vault-section">
        <header>
          <span>Lokaler Vault</span>
          <Vault :size="17" />
        </header>
        <div class="storage-number">
          <strong>{{ vaultStatus?.message_count ?? 0 }}</strong><span>lokale Nachrichten</span>
        </div>
        <div class="storage-track"><span /></div>
        <div class="storage-legend">
          <span><i class="text-data" /> SQLCipher</span>
          <span><i class="media-data" /> FTS5</span>
          <span><i class="free-data" /> {{ nativeVaultRuntime ? "Nativ" : "RAM" }}</span>
        </div>
      </section>

      <section class="detail-section device-section">
        <header>
          <span>Eigene Geräte</span>
          <MonitorSmartphone :size="17" />
        </header>
        <div class="device-row">
          <span class="device-icon"><MonitorSmartphone :size="17" /></span>
          <span><strong>Workstation</strong><small>Dieses Gerät</small></span>
          <span class="verified-dot" title="Verifiziert" />
        </div>
        <div class="device-row muted">
          <span class="device-icon"><FileText :size="17" /></span>
          <span><strong>Recovery</strong><small>Export aktuell</small></span>
          <ShieldCheck :size="15" />
        </div>
      </section>

      <section class="detail-section video-section">
        <header>
          <span>Video</span>
          <Video :size="17" />
        </header>
        <div class="video-actions">
          <button
            type="button"
            :class="{ active: videoStatus?.camera_enabled }"
            :disabled="!canUseVideo"
            :aria-label="videoButtonLabel"
            :title="videoButtonLabel"
            @click="toggleCamera"
          >
            <Video :size="15" />
            Kamera
          </button>
          <button
            type="button"
            :class="{ active: videoStatus?.screen_share_enabled }"
            :disabled="!canUseVideo"
            :aria-label="screenShareButtonLabel"
            :title="screenShareButtonLabel"
            @click="toggleScreenShare"
          >
            <MonitorSmartphone :size="15" />
            Bildschirm
          </button>
        </div>
        <div class="video-preview" :class="videoCaptureState">
          <video
            ref="videoPreview"
            autoplay
            muted
            playsinline
            class="video-preview-player"
          />
          <span class="video-preview-label">{{ videoStatusLabel }}</span>
        </div>
        <span v-if="videoError" class="video-error" role="alert">{{ videoError }}</span>
      </section>

      <section class="detail-section member-section">
        <header>
          <span>Online · 4</span>
          <Users :size="17" />
        </header>
        <div class="member-row"><span class="avatar small mara">MA</span><span><strong>Mara</strong><small>Architecture</small></span></div>
        <div class="member-row"><span class="avatar small jonas">JO</span><span><strong>Jonas</strong><small>Protocol</small></span></div>
        <div class="member-row"><span class="avatar small lea">LE</span><span><strong>Lea</strong><small>Client</small></span></div>
        <div class="member-row"><span class="avatar small noah">NO</span><span><strong>Noah</strong><small>Infrastructure</small></span></div>
      </section>
    </aside>

    <button
      v-if="navigationOpen"
      class="nav-scrim"
      type="button"
      aria-label="Kanäle schließen"
      @click="navigationOpen = false"
    />

    <div
      v-if="vaultGateVisible"
      class="lock-screen"
      role="dialog"
      aria-modal="true"
      aria-labelledby="lock-title"
      :aria-busy="vaultBusy || vaultLoading"
    >
      <div class="lock-mark"><LockKeyhole :size="30" /></div>
      <img :src="singularisLogo" alt="Singularis" class="lock-wordmark" />
      <h2 id="lock-title">{{ vaultGateTitle }}</h2>
      <p v-if="nativeVaultRuntime">{{ vaultGateCopy }}</p>
      <p v-else>Die Browser-Vorschau hält lokale Nachrichten ausschließlich bis zum Neuladen im RAM.</p>

      <form
        v-if="nativeVaultRuntime && !vaultLoading && vaultStatus"
        class="vault-gate-form"
        @submit.prevent="submitVaultGate"
      >
        <label>
          <span>Passphrase</span>
          <input
            v-model="vaultPassphrase"
            type="password"
            :autocomplete="vaultIsUninitialized ? 'new-password' : 'current-password'"
            minlength="12"
            maxlength="1024"
            required
            autofocus
          />
        </label>
        <label v-if="vaultIsUninitialized">
          <span>Passphrase bestätigen</span>
          <input
            v-model="vaultConfirmation"
            type="password"
            autocomplete="new-password"
            minlength="12"
            maxlength="1024"
            required
          />
        </label>
        <span v-if="vaultError" class="vault-error" role="alert">{{ vaultError }}</span>
        <button type="submit" :disabled="vaultBusy">
          {{ vaultBusy ? "Vault wird geöffnet …" : vaultIsUninitialized ? "Vault erstellen" : "Vault entsperren" }}
        </button>
      </form>

      <button v-else-if="!nativeVaultRuntime" type="button" @click="unlockPreview">
        RAM-Vorschau entsperren
      </button>
      <button v-else-if="!vaultLoading" type="button" @click="refreshVaultState">Erneut prüfen</button>
      <span v-if="vaultError && (!vaultStatus || vaultLoading)" class="vault-error" role="alert">
        {{ vaultError }}
      </span>
    </div>
  </div>
</template>