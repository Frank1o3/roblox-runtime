# Observed JNI surface

This is the initial behavior inventory for the Rust `jnivm` work. It was
extracted from one `rusty-blox --host-libc` session supplied with the runtime
workspace. That run reached Roblox's `APP_READY Home` notifications and
`gameLoadedCallback`, so it is useful evidence for the bootstrap and early game
surface. It is one client build and one run, not a complete JNI conformance
trace.

## Counts from the log

- 33 distinct classes reached through `FindClass`.
- 171 distinct class/member/signature records reported as found.
- 58 distinct Java methods called (after deduplicating repeated calls).
- 33 distinct field getter/setter signatures invoked.
- 13 distinct unresolved or unknown Java members (14 log records).
- Calls came from more than one native thread; `GetEnv` and
  `AttachCurrentThread` were both observed.

The `[JNIVM]` lines report class lookup, member resolution and Java-level
dispatch. They do not enumerate every `JNIEnv` function-table slot Roblox used,
so they guide class/member compatibility work but cannot alone define the
complete VM ABI needed for a Rust replacement.

## Classes reached

```text
android/app/ActivityThread
android/content/Context
android/content/res/Configuration
android/os/Build
android/os/Build$VERSION
android/os/LocaleList
android/view/KeyEvent
android/view/MotionEvent
androidx/core/graphics/Insets
androidx/core/view/WindowInsetsCompat$Type
com/google/androidgamesdk/GameActivity
com/google/androidgamesdk/gametextinput/InputConnection
com/google/androidgamesdk/gametextinput/State
com/roblox/audio/AppRtcDeviceWrapper
com/roblox/client/LocalStorageManager
com/roblox/client/flags/NativeFlagsInitResult
com/roblox/engine/jni/NativeGLJavaInterface
com/roblox/engine/jni/locale/NativeLocaleJavaInterface
com/roblox/engine/jni/memstorage/Connection
com/roblox/engine/jni/model/NativeTextBoxInfo
com/roblox/engine/jni/reporter/SessionReporterJavaInterface
com/roblox/engine/jni/user/NativeUserJavaInterface
com/roblox/engine/jni/util/NetworkUtils
com/roblox/engine/jni/video/MediaCodecInfoUtils
com/roblox/universalapp/logging/LoggingProtocol
com/roblox/universalapp/messagebus/Connection
com/snapchat/djinni/NativeObjectManager
java/lang/ClassLoader
java/util/Locale
org/fmod/AudioDevice
org/fmod/FMOD
org/fmod/MediaCodec
org/webrtc/voiceengine/WebRtcAudioManager
```

Additional classes appear in method/field records without a corresponding
`FindClass` line in this log, including `Application`, `SharedPreferences`,
`Resources`, `DisplayMetrics`, `DeviceStaticParams`, `PlatformParams`,
`InitParams`, `StartAppParams`, and `NativeHelper`.

## Unresolved or unknown operations

```text
java/lang/Class.getClassLoader()Ljava/lang/ClassLoader;                  (called unknown)
java/lang/Class.getClassLoader()Ljava/lang/ClassLoader;                  (unresolved member)
android/os/Build.MANUFACTURER:Ljava/lang/String;                         (unknown field getter)
android/os/Build.MANUFACTURER:Ljava/lang/String;                         (unresolved field)
com/google/androidgamesdk/GameActivity.finish()V
com/google/androidgamesdk/GameActivity.getWindowInsets(I)Landroidx/core/graphics/Insets;
com/roblox/engine/jni/NativeGLJavaInterface.getMobileAdvertisingId()V
com/roblox/engine/jni/NativeGLJavaInterface.onExtendedAnalyticsRecvCallback([BI)V
com/roblox/engine/jni/NativeGLJavaInterface.onVrSessionStateUpdate(I)V
com/roblox/engine/jni/NativeGLJavaInterface.promptNativePurchase(JLjava/lang/String;)V
com/roblox/engine/jni/NativeGLJavaInterface.promptNativePurchaseWithPaymentSessionId(JLjava/lang/String;Ljava/lang/String;)V
com/roblox/engine/jni/NativeGLJavaInterface.promptNativePurchaseWithPaymentSessionId(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V
com/roblox/engine/jni/NativeGLJavaInterface.saveImageToAlbum(Ljava/lang/String;)V
```

`getClassLoader` was reported unknown on both call records and was followed by
`ClassLoader.loadClass`/`findClass` activity. The log does not establish that
the unresolved operations are harmless in other flows; purchase, analytics,
VR, media and alternate startup paths need separate observations.

## Remaining observed compatibility work

The previous priorities for JavaVM attachment, member lookup, and string
handling now have initial Rust implementations. The unresolved operations above
still have no Rust handlers; placeholder method calls log the full member and
return descriptor-correct defaults. `java/lang/Class.getClassLoader` is the
first useful next hook because the recorded run followed it with class-loader
lookups. `android/os/Build.MANUFACTURER` also needs a field value. The
GameActivity and NativeGL callbacks need intentional behavior or a documented
no-op based on fresh runtime observations.

The earlier surface inventory above is from the C++ backend. Experimental
startup was also run after the `jni-sys 0.4.1` upgrade with
`USE_EXPERIMENTAL_JNIVM=1`. The Rust VM initialized, called `JNI_OnLoad`, ran
the runtime's startup settings and native flag callbacks, and initialized
GameActivity. Its log is `/tmp/roblox-runtime-experimental-jni-10.log` in the
development environment. This run still crashed after `[stub]
ZSTD_trace_decompress_begin`.

The new run confirmed requests for `java/lang/Class.getClassLoader`,
`java/lang/ClassLoader.loadClass`, and `findClass`; the Rust VM now implements
these like `native/game_activity.cpp` does, normalizing dotted names and
resolving them through the VM. The same trace reaches
`NativeGLJavaInterface.getDeviceStaticParams`; Rust now returns a populated
`DeviceStaticParams` object using the defaults in
`native/android_classes.cpp` (with `RBX_RUNTIME_DEVICE_PROFILE` and
`RBX_RUNTIME_DEVICE_NAME` available for the device strings).

The latest trace reached `GameActivity` initialization but stopped after three
`GetObjectClass on untracked reference` warnings for the same non-null handle.
The experimental path previously passed a `GameActivity` object allocated by
the separate C++ compatibility VM into the engine while the engine used the
Rust JNI table. The experimental path now allocates the Activity, path strings,
asset manager, and configuration in the Rust VM and registers the startup
handlers on `com/google/androidgamesdk/GameActivity`. The C++ backend keeps its
existing initialization path.

The separate C++ compatibility VM still serves other runtime-owned direct
calls. Objects crossing from that VM into Rust JNI remain unsupported, so
other `GetObjectClass on untracked reference` warnings may still identify
bridge gaps. The latest trace has no fatal error or stack trace, and the
`ZSTD_trace_decompress_begin` stub line alone does not establish a crash cause.

## Rust crate status

The Rust modules include JNI and JavaVM ABI tables, per-thread attachment,
class/method/field lookup, native registration, local/global references,
strings, and method dispatch. The `A` method-call entries decode arguments by
descriptor. The `V` call entries pass the platform `va_list` through a small
native ABI decoder and convert its values to the same typed arguments before
Rust dispatch. Java compatibility handlers are incomplete, and the Rust
backend remains opt-in through `USE_EXPERIMENTAL_JNIVM=true`.

When that backend is selected, Roblox receives the Rust JavaVM. The runtime also
starts a separate C++ libjnivm VM for Cordial's existing compatibility classes,
which runtime-owned direct calls still use during startup and app-bridge setup.
Objects allocated by that companion VM are not interchangeable with Rust VM
objects; the experimental GameActivity initialization path now avoids crossing
that boundary for its input references.

## 2026-09-29 startup trace and direct-buffer fix

The 17:36 experimental run reached the system-dialog proxy lookups and then
crashed with a null indirect call. The core dump's call site used JNI table
offset `0x728`, which is `NewDirectByteBuffer` in `jni-sys 0.4.1`'s
`JNINativeInterface_`. The C++ reference at
`third_party/libjnivm/src/jnivm/internal/bytebuffer.cpp` wraps the supplied
address and capacity in a `java/nio/ByteBuffer`; its VM installs that slot and
the matching address and capacity accessors. Rust had left those three table
entries null. The Rust VM now stores the same address and capacity and
implements `NewDirectByteBuffer`, `GetDirectBufferAddress`, and
`GetDirectBufferCapacity`; the flags-loaded callback also reports the buffer
capacity, matching `native/init_params.cpp`.

The next experimental run built from the nested submodule and passed that
crash point. It logged `flags loaded (1339695 bytes)`, then reached the app
ready callbacks for `PlatformAccountRouter`, `Startup`, `Landing`, and `Login`.
This verifies progress through startup and the login screen, not that joining
an experience works. The trace still contains a `NewGlobalRef` failure for an
untracked object and placeholder system-theme methods. It also logs
`ZSTD_trace_compress_begin`; the earlier `ZSTD_trace_decompress_begin` line
preceded the direct-buffer call, and the core dump identified the null JNI slot
as the actual crash site.

The latest startup run reached app initialization, then the APK requested the
Kotlin singleton field
`com/roblox/protocols/systemdialog/PlatformSystemDialogHandler.INSTANCE`.
Neither C++ reference source contains this class, so the Rust VM now supplies a
non-null opaque, VM-global singleton for the observed static field and returns
a thread-local JNI reference when that field is read. The class's dialog
methods remain unimplemented; their behavior must be driven by later observed
calls or an authoritative implementation, not inferred from the field name.

The C++ `SystemClass` in `native/local_storage.cpp` implements
`java/lang/System.identityHashCode(Object)` by deriving a stable value from
object identity. The Rust VM now handles the observed method from its stable
object handles instead of passing these Rust-only objects to the C++ fallback.
