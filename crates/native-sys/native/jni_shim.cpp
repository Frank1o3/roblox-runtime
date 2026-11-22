// Temporary JNI compatibility VM surface used by the native hook drivers.
// The Rust JavaVM is owned by crates/jnivm.

#include <jnivm.h>

extern "C" void roblox_register_android_classes(void* env);
namespace roblox_runtime {
void register_game_activity_classes(jnivm::ENV* env);
void register_init_params_classes(jnivm::ENV* env);
}

/// The process VM, for translation units that need the real jnivm::ENV rather
/// than a JNIEnv. Keeping this on the C++ side means Rust never has to hold a
/// type it cannot name — passing a JNIEnv* and casting it to jnivm::ENV* is a
/// silent type confusion that surfaces as a null function pointer several calls
/// later.
namespace roblox_runtime { jnivm::ENV* process_env(); }

#include <cstddef>
#include <cstdio>

extern "C" int roblox_runtime_log_level();
#include <cstdlib>
#include <cstdarg>
#include <exception>
#include <stdexcept>
#include <memory>
#include <mutex>
#include <algorithm>
#include <cstdint>
#include <cstring>

namespace {
// The VM owns every class, object and environment it hands out, so it outlives
// anything derived from it. One per process.
std::unique_ptr<jnivm::VM> g_vm;
JavaVM* g_compat_vm = nullptr;

/// Stand up libjnivm's compatibility classes independently of the JavaVM
/// Roblox receives. The experimental Rust VM owns that JavaVM, while existing
/// roblox-runtime Java hooks still use libjnivm objects when the runtime invokes
/// exported engine natives directly (GameActivity/app-bridge startup).
bool initialize_compat_vm() {
    if (g_vm) {
        return true;
    }
    g_vm = std::make_unique<jnivm::VM>();
    roblox_register_android_classes(g_vm->GetEnv().get());
    roblox_runtime::register_game_activity_classes(g_vm->GetEnv().get());
    roblox_runtime::register_init_params_classes(g_vm->GetEnv().get());
    g_compat_vm = g_vm->GetJavaVM();
    return true;
}

} // namespace

extern "C" {

/// Ensure the C++ Java compatibility bridge is available while the engine's
/// JavaVM is provided by the experimental Rust implementation.
int roblox_jni_init_compat_bridge() {
    try {
        if (!initialize_compat_vm()) {
            return -1;
        }
        fprintf(stderr, "[jni] C++ Java compatibility bridge initialized\n");
        return 0;
    } catch (const std::exception& e) {
        fprintf(stderr, "[jni] could not initialize C++ compatibility bridge: %s\n", e.what());
        return -1;
    } catch (...) {
        fprintf(stderr, "[jni] could not initialize C++ compatibility bridge: non-standard exception\n");
        return -1;
    }
}

/// Decode `Call*MethodV` arguments using the host C ABI. The JNI contract uses
/// the platform's `va_list` representation, which Rust does not expose in a
/// portable stable form.
int roblox_jni_decode_va_list(const char* signature, va_list args,
                              jvalue* output, int capacity) {
    if (!signature || capacity < 0) return -1;
    const char* cursor = std::strchr(signature, '(');
    if (!cursor) return -1;
    ++cursor;
    if (*cursor == ')') return 0;
    if (!args || !output) return -2;

    va_list copied;
    va_copy(copied, args);
    int count = 0;
    while (*cursor && *cursor != ')') {
        if (count >= capacity) {
            va_end(copied);
            return -3;
        }
        jvalue value{};
        const char kind = *cursor++;
        switch (kind) {
        case 'Z': case 'B': case 'C': case 'S': case 'I': {
            // Narrow JNI primitives undergo C's default argument promotion.
            const jint promoted = va_arg(copied, jint);
            switch (kind) {
            case 'Z': value.z = static_cast<jboolean>(promoted); break;
            case 'B': value.b = static_cast<jbyte>(promoted); break;
            case 'C': value.c = static_cast<jchar>(promoted); break;
            case 'S': value.s = static_cast<jshort>(promoted); break;
            default: value.i = promoted; break;
            }
            break;
        }
        case 'J':
            value.j = va_arg(copied, jlong);
            break;
        case 'F':
            // `float` is promoted to `double` in a C variadic call.
            value.f = static_cast<jfloat>(va_arg(copied, double));
            break;
        case 'D':
            value.d = va_arg(copied, jdouble);
            break;
        case 'L': {
            const char* end = std::strchr(cursor, ';');
            if (!end) {
                va_end(copied);
                return -1;
            }
            cursor = end + 1;
            value.l = va_arg(copied, jobject);
            break;
        }
        case '[':
            while (*cursor == '[') ++cursor;
            if (*cursor == 'L') {
                const char* end = std::strchr(cursor, ';');
                if (!end) {
                    va_end(copied);
                    return -1;
                }
                cursor = end + 1;
            } else if (*cursor && std::strchr("ZBCSIJFD", *cursor)) {
                ++cursor;
            } else {
                va_end(copied);
                return -1;
            }
            value.l = va_arg(copied, jobject);
            break;
        default:
            va_end(copied);
            return -1;
        }
        output[count++] = value;
    }
    const bool complete = *cursor == ')';
    va_end(copied);
    return complete && count == capacity ? 0 : -1;
}

/// Mirror an engine `RegisterNatives` entry for the remaining native
/// GameActivity drivers that query the compatibility VM's native table.
int roblox_jni_register_native(const char* class_name, const char* name,
                               const char* signature, void* function) {
    try {
        if (!g_vm || !class_name || !name || !signature || !function) return -1;
        auto* env = roblox_runtime::process_env();
        if (!env) return -1;
        auto cls = env->GetClass(class_name);
        if (!cls) return -1;
        std::lock_guard<std::mutex> lock(cls->mtx);
        cls->natives[name] = function;
        return 0;
    } catch (...) {
        return -1;
    }
}

/// Classify a jobject owned by the compatibility VM without dereferencing
/// an unknown pointer. Kind values: 1 current-thread local, 2 global, 3 class.
int roblox_jni_compat_reference_kind(jobject object) {
    if (!g_vm || !object) return 0;
    try {
        auto* target = reinterpret_cast<jnivm::Object*>(object);
        {
            std::lock_guard<std::mutex> lock(g_vm->mtx);
            for (const auto& entry : g_vm->classes) {
                if (entry.second.get() == target) return 3;
            }
            for (const auto& global : g_vm->globals) {
                if (global.get() == target) return 2;
            }
        }
        auto* env = roblox_runtime::process_env();
        if (!env) return 0;
#ifdef EnableJNIVMGC
        for (const auto& frame : env->localframe) {
            for (const auto& local : frame) {
                if (local.get() == target) return 1;
            }
        }
#endif
        return 0;
    } catch (...) {
        return 0;
    }
}

jint roblox_jni_compat_push_local_frame(jint capacity) {
    if (!g_vm) return JNI_OK;
    try {
        auto* env = roblox_runtime::process_env();
        return env ? env->GetJNIEnv()->PushLocalFrame(capacity) : JNI_ERR;
    } catch (...) {
        return JNI_ERR;
    }
}

jobject roblox_jni_compat_pop_local_frame(jobject result) {
    try {
        auto* env = roblox_runtime::process_env();
        return env ? env->GetJNIEnv()->PopLocalFrame(result) : result;
    } catch (...) {
        return result;
    }
}

jobject roblox_jni_compat_new_global_ref(jobject object) {
    if (roblox_jni_compat_reference_kind(object) == 0) return nullptr;
    try {
        auto* env = roblox_runtime::process_env();
        return env ? env->GetJNIEnv()->NewGlobalRef(object) : nullptr;
    } catch (...) {
        return nullptr;
    }
}

jobject roblox_jni_compat_new_local_ref(jobject object) {
    if (roblox_jni_compat_reference_kind(object) == 0) return nullptr;
    try {
        auto* env = roblox_runtime::process_env();
        return env ? env->GetJNIEnv()->NewLocalRef(object) : nullptr;
    } catch (...) {
        return nullptr;
    }
}

void roblox_jni_compat_delete_global_ref(jobject object) {
    if (roblox_jni_compat_reference_kind(object) != 2) return;
    try {
        if (auto* env = roblox_runtime::process_env()) {
            env->GetJNIEnv()->DeleteGlobalRef(object);
        }
    } catch (...) {
    }
}

void roblox_jni_compat_delete_local_ref(jobject object) {
    if (roblox_jni_compat_reference_kind(object) != 1) return;
    try {
        if (auto* env = roblox_runtime::process_env()) {
            env->GetJNIEnv()->DeleteLocalRef(object);
        }
    } catch (...) {
    }
}

jboolean roblox_jni_compat_is_same_object(jobject left, jobject right) {
    if (roblox_jni_compat_reference_kind(left) == 0 ||
        roblox_jni_compat_reference_kind(right) == 0) return JNI_FALSE;
    try {
        auto* env = roblox_runtime::process_env();
        return env ? env->GetJNIEnv()->IsSameObject(left, right) : JNI_FALSE;
    } catch (...) {
        return JNI_FALSE;
    }
}

/// Decode JNI's `Call*MethodV` arguments with the host C ABI. Rust can safely
/// dispatch the resulting `jvalue`s, but `va_list` itself is target-specific
int roblox_jni_dump_classes(const char* path) {
#ifdef JNI_DEBUG
    if (!g_vm) {
        return -1;
    }
    g_vm->GenerateClassDump(path);
    return 0;
#else
    (void)path;
    return -2;  // built without JNI_DEBUG
#endif
}


} // extern "C"

namespace roblox_runtime {
jnivm::ENV* process_env() {
    if (!g_vm || !g_compat_vm) {
        return nullptr;
    }
    // NOT `g_vm->GetEnv()`. That is `jnienvs[pthread_self()]` — an
    // unordered_map::operator[], which for a thread it has never seen
    // default-constructs a null shared_ptr and returns a reference to it. Every
    // JNI hook in this build calls process_env(), and the engine calls those
    // hooks from threads it created itself, so on those threads the old version
    // handed back a null ENV and the engine stored the null where it expected a
    // JNIEnv.
    //
    // AttachCurrentThread is the entry point that does create an env for an
    // unknown thread; on a thread that already has one it finds it and changes
    // nothing. Going through it makes "which thread am I on" stop mattering.
    JNIEnv* env = nullptr;
    if (g_compat_vm->AttachCurrentThread(&env, nullptr) != JNI_OK || !env) {
        return nullptr;
    }
    return jnivm::ENV::FromJNIEnv(env);
}
} // namespace roblox_runtime
