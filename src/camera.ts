// The app-owned camera window for the desktop photo chooser (see
// src-tauri/src/photo_capture.rs). It may use the camera only while the
// capture session that opened it is live, takes one photo, hands it to the app
// and closes. Archive pages never reach this window.
import { invoke } from "@tauri-apps/api/core";
import { setLocale, t, whenLocaleReady, localeDirection } from "./i18n";

const params = new URLSearchParams(location.search);
const token = params.get("token") ?? "";
const facing = params.get("facing") === "user" ? "user" : "environment";
// JPEG, unless the page accepts PNG but not JPEG.
const format = params.get("format") === "png" ? "image/png" : "image/jpeg";
const MAX_SIDE = 4096;

const video = document.getElementById("preview") as HTMLVideoElement;
const message = document.getElementById("message") as HTMLParagraphElement;
const take = document.getElementById("take") as HTMLButtonElement;
const cancel = document.getElementById("cancel") as HTMLButtonElement;
const flip = document.getElementById("switch") as HTMLButtonElement;

let stream: MediaStream | null = null;
let devices: MediaDeviceInfo[] = [];
let deviceIndex = -1;
let finished = false;

function stop() {
  for (const track of stream?.getTracks() ?? []) track.stop();
  stream = null;
  video.srcObject = null;
}

function say(text: string) {
  message.textContent = text;
  message.hidden = !text;
}

function failure(error: unknown): string {
  const name = error instanceof DOMException ? error.name : "";
  if (name === "NotAllowedError" || name === "SecurityError")
    return t("Kiwix cannot use the camera. In Windows Settings, open Privacy & security › Camera and allow desktop apps to use it.");
  if (name === "NotFoundError" || name === "OverconstrainedError")
    return t("No camera was found. Cancel and choose a file instead.");
  if (name === "NotReadableError")
    return t("The camera is in use by another app. Close it, then try again.");
  return t("The camera could not start. Cancel and choose a file instead.");
}

async function start(deviceId?: string) {
  stop();
  take.disabled = true;
  say(t("Starting the camera…"));
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: deviceId
        ? { deviceId: { exact: deviceId }, width: { ideal: 1920 }, height: { ideal: 1080 } }
        : { facingMode: facing, width: { ideal: 1920 }, height: { ideal: 1080 } },
    });
    video.srcObject = stream;
    await video.play().catch(() => undefined);
    const settings = stream.getVideoTracks()[0]?.getSettings() ?? {};
    // A front (user-facing) camera previews as a mirror, like a mirror would;
    // the photo itself is not mirrored.
    video.classList.toggle("mirror", settings.facingMode === "user" || (!settings.facingMode && facing === "user"));
    devices = (await navigator.mediaDevices.enumerateDevices()).filter((device) => device.kind === "videoinput");
    deviceIndex = devices.findIndex((device) => device.deviceId === settings.deviceId);
    flip.hidden = devices.length < 2;
    say("");
    take.disabled = false;
    take.focus();
  } catch (error) {
    stop();
    say(failure(error));
    cancel.focus();
  }
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  return btoa(binary);
}

async function capture() {
  if (finished || !stream || !video.videoWidth || !video.videoHeight) return;
  take.disabled = true;
  const scale = Math.min(1, MAX_SIDE / Math.max(video.videoWidth, video.videoHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(video.videoWidth * scale);
  canvas.height = Math.round(video.videoHeight * scale);
  canvas.getContext("2d")?.drawImage(video, 0, 0, canvas.width, canvas.height);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, format, 0.92));
  stop();
  if (!blob) {
    say(t("The photo could not be taken. Try again."));
    void start();
    return;
  }
  finished = true;
  say(t("Sending the photo…"));
  try {
    await invoke("dispatch", {
      command: "camera_capture_complete",
      args: { token, data: base64(new Uint8Array(await blob.arrayBuffer())) },
    });
  } catch (error) {
    say(String(error));
  }
}

async function close(reason?: string) {
  if (finished) return;
  finished = true;
  stop();
  try {
    await invoke("dispatch", { command: "camera_capture_cancel", args: { token, reason: reason ?? null } });
  } catch {
    window.close();
  }
}

take.addEventListener("click", () => void capture());
cancel.addEventListener("click", () => void close());
flip.addEventListener("click", () => {
  if (devices.length < 2) return;
  deviceIndex = (deviceIndex + 1) % devices.length;
  void start(devices[deviceIndex].deviceId);
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") void close();
});
window.addEventListener("pagehide", stop);

setLocale(params.get("locale") || "system");
void whenLocaleReady().then(() => {
  document.documentElement.lang = params.get("locale") || document.documentElement.lang;
  document.documentElement.dir = localeDirection();
  document.title = t("Kiwix — Camera");
  take.textContent = t("Take photo");
  cancel.textContent = t("Cancel");
  flip.textContent = t("Switch camera");
  video.setAttribute("aria-label", t("Camera preview"));
  void start();
});
