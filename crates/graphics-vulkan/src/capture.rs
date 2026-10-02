//! Swapchain readback for the single host window used by the embedding client.
//!
//! The bridge adds transfer-source usage at swapchain creation, then chains a
//! copy submission in front of presentation. The app's present semaphores are
//! consumed by the copy submission; a per-swapchain-image semaphore is passed
//! to presentation and is reused only after that image is acquired again.

use ash::vk::{self, Handle};
use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

const FRAME_QUEUE_CAPACITY: usize = 3;

static ENABLED: AtomicBool = AtomicBool::new(false);
static DEVICES: OnceLock<Mutex<HashMap<u64, DeviceCapture>>> = OnceLock::new();
static FRAMES: OnceLock<Mutex<VecDeque<CapturedFrame>>> = OnceLock::new();

static HOST_QUEUE_PRESENT: AtomicUsize = AtomicUsize::new(0);
static HOST_GET_SWAPCHAIN_IMAGES: AtomicUsize = AtomicUsize::new(0);
static HOST_ACQUIRE_NEXT_IMAGE: AtomicUsize = AtomicUsize::new(0);
static HOST_ACQUIRE_NEXT_IMAGE2: AtomicUsize = AtomicUsize::new(0);
static HOST_GET_DEVICE_QUEUE: AtomicUsize = AtomicUsize::new(0);
static HOST_GET_DEVICE_QUEUE2: AtomicUsize = AtomicUsize::new(0);
static HOST_DESTROY_SWAPCHAIN: AtomicUsize = AtomicUsize::new(0);
static HOST_DESTROY_DEVICE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed BGR8 pixels, suitable for OpenCV's CV_8UC3 convention.
    pub bgr: Vec<u8>,
}

struct DeviceCapture {
    device: ash::Device,
    memory: vk::PhysicalDeviceMemoryProperties,
    queues: HashMap<u64, u32>,
    queue_flags: Vec<vk::QueueFlags>,
    swapchains: HashMap<u64, SwapchainCapture>,
}

struct SwapchainCapture {
    format: vk::Format,
    extent: vk::Extent2D,
    transfer_source: bool,
    images: Vec<vk::Image>,
    slots: Vec<Option<CaptureSlot>>,
}

struct CaptureSlot {
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    allocation_size: vk::DeviceSize,
    coherent: bool,
    fence: vk::Fence,
    present_semaphore: vk::Semaphore,
    pending: bool,
}

struct CaptureSlotBuilder<'a> {
    device: &'a ash::Device,
    command_pool: Option<vk::CommandPool>,
    buffer: Option<vk::Buffer>,
    memory: Option<vk::DeviceMemory>,
    fence: Option<vk::Fence>,
    present_semaphore: Option<vk::Semaphore>,
}

impl Drop for CaptureSlotBuilder<'_> {
    fn drop(&mut self) {
        unsafe {
            if let Some(semaphore) = self.present_semaphore.take() {
                self.device.destroy_semaphore(semaphore, None);
            }
            if let Some(fence) = self.fence.take() {
                self.device.destroy_fence(fence, None);
            }
            if let Some(buffer) = self.buffer.take() {
                self.device.destroy_buffer(buffer, None);
            }
            if let Some(memory) = self.memory.take() {
                self.device.free_memory(memory, None);
            }
            if let Some(pool) = self.command_pool.take() {
                self.device.destroy_command_pool(pool, None);
            }
        }
    }
}

fn devices() -> &'static Mutex<HashMap<u64, DeviceCapture>> {
    DEVICES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn frame_queue() -> &'static Mutex<VecDeque<CapturedFrame>> {
    FRAMES.get_or_init(|| Mutex::new(VecDeque::with_capacity(FRAME_QUEUE_CAPACITY)))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Release);
    if !enabled {
        lock(frame_queue()).clear();
    }
}

pub fn take_frame() -> Option<CapturedFrame> {
    if !ENABLED.load(Ordering::Acquire) {
        lock(frame_queue()).clear();
        return None;
    }
    lock(frame_queue()).pop_front()
}

pub(crate) fn register_device(
    physical: vk::PhysicalDevice,
    device_handle: vk::Device,
    get_device_proc_addr: unsafe extern "system" fn(
        vk::Device,
        *const c_char,
    ) -> vk::PFN_vkVoidFunction,
    get_memory_properties: unsafe extern "system" fn(
        vk::PhysicalDevice,
        *mut vk::PhysicalDeviceMemoryProperties,
    ),
    get_queue_family_properties: unsafe extern "system" fn(
        vk::PhysicalDevice,
        *mut u32,
        *mut vk::QueueFamilyProperties,
    ),
) {
    if physical.is_null() || device_handle.is_null() {
        return;
    }
    // SAFETY: the callback is the host loader's device proc resolver for this live device.
    let device = unsafe {
        ash::Device::load_with(
            |name| {
                get_device_proc_addr(device_handle, name.as_ptr())
                    .map_or(std::ptr::null(), |function| {
                        function as *const () as *const c_void
                    })
            },
            device_handle,
        )
    };
    let mut memory = vk::PhysicalDeviceMemoryProperties::default();
    // SAFETY: the callback is the host Vulkan loader's physical-device query.
    unsafe { get_memory_properties(physical, &mut memory) };
    let mut queue_count = 0;
    // SAFETY: Vulkan's two-call enumeration obtains the queue-family count.
    unsafe { get_queue_family_properties(physical, &mut queue_count, std::ptr::null_mut()) };
    let mut queue_properties = vec![vk::QueueFamilyProperties::default(); queue_count as usize];
    if queue_count > 0 {
        // SAFETY: output has the count returned by the first call.
        unsafe {
            get_queue_family_properties(physical, &mut queue_count, queue_properties.as_mut_ptr())
        };
        queue_properties.truncate(queue_count as usize);
    }
    lock(devices()).insert(
        device_handle.as_raw(),
        DeviceCapture {
            device,
            memory,
            queues: HashMap::new(),
            queue_flags: queue_properties
                .iter()
                .map(|props| props.queue_flags)
                .collect(),
            swapchains: HashMap::new(),
        },
    );
}

pub(crate) fn register_queue(device: vk::Device, family: u32, queue: vk::Queue) {
    if device.is_null() || queue.is_null() {
        return;
    }
    if let Some(state) = lock(devices()).get_mut(&device.as_raw()) {
        state.queues.insert(queue.as_raw(), family);
    }
}

pub(crate) fn register_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    format: vk::Format,
    extent: vk::Extent2D,
    transfer_source: bool,
) {
    if device.is_null() || swapchain.is_null() {
        return;
    }
    lock(devices()).entry(device.as_raw()).and_modify(|state| {
        state.swapchains.insert(
            swapchain.as_raw(),
            SwapchainCapture {
                format,
                extent,
                transfer_source,
                images: Vec::new(),
                slots: Vec::new(),
            },
        );
    });
}

pub(crate) fn intercept_device_proc(
    device: vk::Device,
    name: &CStr,
    host: vk::PFN_vkVoidFunction,
) -> vk::PFN_vkVoidFunction {
    if device.is_null() {
        return host;
    }
    let Some(host) = host else { return None };
    let address = host as *const () as usize;
    let (slot, wrapper) = match name.to_bytes() {
        b"vkQueuePresentKHR" => (
            &HOST_QUEUE_PRESENT,
            queue_present as *const () as *mut c_void,
        ),
        b"vkGetSwapchainImagesKHR" => (
            &HOST_GET_SWAPCHAIN_IMAGES,
            get_swapchain_images as *const () as *mut c_void,
        ),
        b"vkAcquireNextImageKHR" => (
            &HOST_ACQUIRE_NEXT_IMAGE,
            acquire_next_image as *const () as *mut c_void,
        ),
        b"vkAcquireNextImage2KHR" => (
            &HOST_ACQUIRE_NEXT_IMAGE2,
            acquire_next_image2 as *const () as *mut c_void,
        ),
        b"vkGetDeviceQueue" => (
            &HOST_GET_DEVICE_QUEUE,
            get_device_queue as *const () as *mut c_void,
        ),
        b"vkGetDeviceQueue2" => (
            &HOST_GET_DEVICE_QUEUE2,
            get_device_queue2 as *const () as *mut c_void,
        ),
        b"vkDestroySwapchainKHR" => (
            &HOST_DESTROY_SWAPCHAIN,
            destroy_swapchain as *const () as *mut c_void,
        ),
        b"vkDestroyDevice" => (
            &HOST_DESTROY_DEVICE,
            destroy_device as *const () as *mut c_void,
        ),
        _ => return Some(host),
    };
    slot.store(address, Ordering::Release);
    // SAFETY: the selected wrapper has the Vulkan ABI and exact signature for this command.
    Some(unsafe { std::mem::transmute(wrapper) })
}

unsafe extern "system" fn get_swapchain_images(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    count: *mut u32,
    images: *mut vk::Image,
) -> vk::Result {
    let address = HOST_GET_SWAPCHAIN_IMAGES.load(Ordering::Acquire);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkGetSwapchainImagesKHR = unsafe { std::mem::transmute(address) };
    // SAFETY: Vulkan's two-call enumeration API validates the paired count/output arguments.
    let result = unsafe { function(device, swapchain, count, images) };
    if result == vk::Result::SUCCESS && !count.is_null() && !images.is_null() {
        // SAFETY: on success Vulkan wrote `*count` image handles into the provided array.
        let returned = unsafe { std::slice::from_raw_parts(images, *count as usize) };
        if let Some(state) = lock(devices()).get_mut(&device.as_raw()) {
            if let Some(swap) = state.swapchains.get_mut(&swapchain.as_raw()) {
                swap.images.clear();
                swap.images.extend_from_slice(returned);
                swap.slots.resize_with(returned.len(), || None);
            }
        }
    }
    result
}

unsafe extern "system" fn get_device_queue(
    device: vk::Device,
    family: u32,
    index: u32,
    output: *mut vk::Queue,
) {
    let address = HOST_GET_DEVICE_QUEUE.load(Ordering::Acquire);
    if address == 0 {
        return;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkGetDeviceQueue = unsafe { std::mem::transmute(address) };
    // SAFETY: preserve the application's original Vulkan arguments.
    unsafe { function(device, family, index, output) };
    if !output.is_null() {
        // SAFETY: vkGetDeviceQueue writes a queue handle to this output pointer.
        register_queue(device, family, unsafe { *output });
    }
}

unsafe extern "system" fn get_device_queue2(
    device: vk::Device,
    info: *const vk::DeviceQueueInfo2<'_>,
    output: *mut vk::Queue,
) {
    let address = HOST_GET_DEVICE_QUEUE2.load(Ordering::Acquire);
    if address == 0 {
        return;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkGetDeviceQueue2 = unsafe { std::mem::transmute(address) };
    // SAFETY: preserve the application's original Vulkan arguments.
    unsafe { function(device, info, output) };
    // SAFETY: the non-null output and info belong to the synchronous caller.
    if let (Some(queue), Some(info)) = (unsafe { output.as_ref() }, unsafe { info.as_ref() }) {
        register_queue(device, info.queue_family_index, *queue);
    }
}

unsafe extern "system" fn acquire_next_image(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    timeout: u64,
    semaphore: vk::Semaphore,
    fence: vk::Fence,
    image_index: *mut u32,
) -> vk::Result {
    let address = HOST_ACQUIRE_NEXT_IMAGE.load(Ordering::Acquire);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkAcquireNextImageKHR = unsafe { std::mem::transmute(address) };
    // SAFETY: preserve the application's original Vulkan arguments.
    let result = unsafe { function(device, swapchain, timeout, semaphore, fence, image_index) };
    if matches!(result, vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR) && !image_index.is_null()
    {
        // SAFETY: successful acquire writes a valid index to `image_index`.
        harvest_image(device, swapchain, unsafe { *image_index });
    }
    result
}

unsafe extern "system" fn acquire_next_image2(
    device: vk::Device,
    info: *const vk::AcquireNextImageInfoKHR<'_>,
    image_index: *mut u32,
) -> vk::Result {
    let address = HOST_ACQUIRE_NEXT_IMAGE2.load(Ordering::Acquire);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkAcquireNextImage2KHR = unsafe { std::mem::transmute(address) };
    // SAFETY: preserve the application's original Vulkan arguments.
    let result = unsafe { function(device, info, image_index) };
    // SAFETY: info and image_index are valid for this synchronous Vulkan call.
    if matches!(result, vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR)
        && let (Some(info), Some(index)) =
            (unsafe { info.as_ref() }, unsafe { image_index.as_ref() })
    {
        harvest_image(device, info.swapchain, *index);
    }
    result
}

unsafe extern "system" fn queue_present(
    queue: vk::Queue,
    info: *const vk::PresentInfoKHR<'_>,
) -> vk::Result {
    let address = HOST_QUEUE_PRESENT.load(Ordering::Acquire);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkQueuePresentKHR = unsafe { std::mem::transmute(address) };
    if !ENABLED.load(Ordering::Acquire) {
        // SAFETY: the caller provided the original valid VkPresentInfoKHR.
        return unsafe { function(queue, info) };
    }
    // SAFETY: the present info is a synchronous input struct supplied by Vulkan's caller.
    let Some(present) = (unsafe { info.as_ref() }) else {
        return unsafe { function(queue, info) };
    };
    if present.swapchain_count != 1
        || present.p_swapchains.is_null()
        || present.p_image_indices.is_null()
    {
        // This initial bridge path deliberately handles the runtime's one window/swapchain.
        return unsafe { function(queue, info) };
    }
    // SAFETY: Vulkan pairs both arrays with swapchain_count.
    let swapchains = unsafe { std::slice::from_raw_parts(present.p_swapchains, 1) };
    // SAFETY: same paired-count contract as above.
    let indices = unsafe { std::slice::from_raw_parts(present.p_image_indices, 1) };
    let swapchain = swapchains[0];
    let image_index = indices[0] as usize;

    let mut states = lock(devices());
    let Some(state) = states
        .values_mut()
        .find(|state| state.queues.contains_key(&queue.as_raw()))
    else {
        return unsafe { function(queue, info) };
    };
    let Some(family) = state.queues.get(&queue.as_raw()).copied() else {
        return unsafe { function(queue, info) };
    };
    let Some(swap) = state.swapchains.get_mut(&swapchain.as_raw()) else {
        return unsafe { function(queue, info) };
    };
    if !swap.transfer_source
        || image_index >= swap.images.len()
        || present.wait_semaphore_count == 0
        || present.p_wait_semaphores.is_null()
        || state.queue_flags.get(family as usize).is_none_or(|flags| {
            !flags.intersects(vk::QueueFlags::GRAPHICS | vk::QueueFlags::TRANSFER)
        })
    {
        return unsafe { function(queue, info) };
    }
    if !supported_capture_format(swap.format) {
        return unsafe { function(queue, info) };
    }
    if swap.slots.len() != swap.images.len() {
        swap.slots.resize_with(swap.images.len(), || None);
    }
    if swap.slots[image_index].is_none() {
        match CaptureSlot::new(&state.device, &state.memory, family, swap.extent) {
            Ok(slot) => swap.slots[image_index] = Some(slot),
            Err(_) => return unsafe { function(queue, info) },
        }
    }
    let Some(slot) = swap.slots[image_index].as_mut() else {
        return unsafe { function(queue, info) };
    };
    if slot.pending {
        return unsafe { function(queue, info) };
    }

    if record_copy(&state.device, slot, swap.images[image_index], swap.extent).is_err() {
        return unsafe { function(queue, info) };
    }
    // The application wait semaphores are consumed here; presentation waits on
    // this image slot's semaphore after the copy has restored PRESENT layout.
    let waits = unsafe {
        std::slice::from_raw_parts(
            present.p_wait_semaphores,
            present.wait_semaphore_count as usize,
        )
    };
    let wait_stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
    let command_buffers = [slot.command_buffer];
    let signals = [slot.present_semaphore];
    let submit = vk::SubmitInfo::default()
        .wait_semaphores(waits)
        .wait_dst_stage_mask(&wait_stages)
        .command_buffers(&command_buffers)
        .signal_semaphores(&signals);
    if unsafe { state.device.queue_submit(queue, &[submit], slot.fence) }.is_err() {
        return unsafe { function(queue, info) };
    }
    slot.pending = true;
    let waits_for_present = [slot.present_semaphore];
    let patched = vk::PresentInfoKHR {
        wait_semaphore_count: 1,
        p_wait_semaphores: waits_for_present.as_ptr(),
        ..*present
    };
    // SAFETY: patched is a shallow copy; its changed semaphore array lives for the call.
    unsafe { function(queue, &patched) }
}

unsafe extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    let address = HOST_DESTROY_SWAPCHAIN.load(Ordering::Acquire);
    if address == 0 {
        return;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkDestroySwapchainKHR = unsafe { std::mem::transmute(address) };
    if let Some(state) = lock(devices()).get_mut(&device.as_raw()) {
        // SAFETY: destroy is called while the device is live; waiting here only occurs at teardown.
        let _ = unsafe { state.device.device_wait_idle() };
        if let Some(mut swap) = state.swapchains.remove(&swapchain.as_raw()) {
            for slot in swap.slots.drain(..).flatten() {
                destroy_slot(&state.device, slot);
            }
        }
    }
    // SAFETY: preserve the application's original Vulkan arguments.
    unsafe { function(device, swapchain, allocator) };
}

unsafe extern "system" fn destroy_device(
    device: vk::Device,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    let address = HOST_DESTROY_DEVICE.load(Ordering::Acquire);
    if address == 0 {
        return;
    }
    // SAFETY: this pointer was resolved from vkGetDeviceProcAddr for this Vulkan command.
    let function: vk::PFN_vkDestroyDevice = unsafe { std::mem::transmute(address) };
    if let Some(mut state) = lock(devices()).remove(&device.as_raw()) {
        // SAFETY: destruction is the point where all queue work and capture resources must finish.
        let _ = unsafe { state.device.device_wait_idle() };
        for (_, mut swap) in state.swapchains.drain() {
            for slot in swap.slots.drain(..).flatten() {
                destroy_slot(&state.device, slot);
            }
        }
    }
    // SAFETY: preserve the application's original Vulkan arguments.
    unsafe { function(device, allocator) };
}

fn harvest_image(device: vk::Device, swapchain: vk::SwapchainKHR, image_index: u32) {
    let mut states = lock(devices());
    let Some(state) = states.get_mut(&device.as_raw()) else {
        return;
    };
    let Some(swap) = state.swapchains.get_mut(&swapchain.as_raw()) else {
        return;
    };
    let index = image_index as usize;
    if index >= swap.slots.len() {
        return;
    }
    let Some(slot) = swap.slots[index].as_mut() else {
        return;
    };
    if !slot.pending {
        return;
    }
    // Reacquiring this image means presentation consumed its semaphore. The
    // capture fence should also be complete; never block the engine if it is not.
    // SAFETY: this is a nonblocking status check on a live fence.
    if unsafe { state.device.get_fence_status(slot.fence) } != Ok(true) {
        return;
    }
    if let Some(frame) = readback_frame(&state.device, swap.format, swap.extent, slot) {
        let mut frames = lock(frame_queue());
        if frames.len() == FRAME_QUEUE_CAPACITY {
            frames.pop_front();
        }
        frames.push_back(frame);
    }
    slot.pending = false;
}

fn supported_capture_format(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_UNORM
            | vk::Format::B8G8R8A8_SRGB
            | vk::Format::R8G8B8A8_UNORM
            | vk::Format::R8G8B8A8_SRGB
    )
}

impl CaptureSlot {
    fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        family: u32,
        extent: vk::Extent2D,
    ) -> Result<Self, vk::Result> {
        let byte_len = (extent.width as u64)
            .checked_mul(extent.height as u64)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(vk::Result::ERROR_OUT_OF_HOST_MEMORY)?;
        let mut builder = CaptureSlotBuilder {
            device,
            command_pool: None,
            buffer: None,
            memory: None,
            fence: None,
            present_semaphore: None,
        };
        builder.command_pool = Some(unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        });
        let command_pool = builder.command_pool.expect("capture command pool created");
        let command_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0]
        };
        builder.buffer = Some(unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(byte_len)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?
        });
        let buffer = builder.buffer.expect("capture buffer created");
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let (memory_type_index, coherent) =
            find_memory_type(memory_properties, requirements.memory_type_bits)
                .ok_or(vk::Result::ERROR_FEATURE_NOT_PRESENT)?;
        let allocation_size = requirements.size;
        builder.memory = Some(unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(allocation_size)
                    .memory_type_index(memory_type_index),
                None,
            )?
        });
        let memory = builder.memory.expect("capture memory allocated");
        unsafe { device.bind_buffer_memory(buffer, memory, 0)? };
        builder.fence =
            Some(unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None)? });
        builder.present_semaphore =
            Some(unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)? });
        Ok(Self {
            command_pool: builder
                .command_pool
                .take()
                .expect("capture command pool created"),
            command_buffer,
            buffer: builder.buffer.take().expect("capture buffer created"),
            memory: builder.memory.take().expect("capture memory allocated"),
            allocation_size,
            coherent,
            fence: builder.fence.take().expect("capture fence created"),
            present_semaphore: builder
                .present_semaphore
                .take()
                .expect("capture semaphore created"),
            pending: false,
        })
    }
}

fn find_memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    allowed: u32,
) -> Option<(u32, bool)> {
    let mut fallback = None;
    for index in 0..properties.memory_type_count {
        if allowed & (1 << index) == 0 {
            continue;
        }
        let flags = properties.memory_types[index as usize].property_flags;
        if !flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) {
            continue;
        }
        let coherent = flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT);
        if coherent {
            return Some((index, true));
        }
        fallback.get_or_insert((index, false));
    }
    fallback
}

fn record_copy(
    device: &ash::Device,
    slot: &mut CaptureSlot,
    image: vk::Image,
    extent: vk::Extent2D,
) -> Result<(), vk::Result> {
    unsafe {
        device.reset_fences(&[slot.fence])?;
        device.reset_command_buffer(slot.command_buffer, vk::CommandBufferResetFlags::empty())?;
        device.begin_command_buffer(
            slot.command_buffer,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        let to_transfer = [vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            )];
        device.cmd_pipeline_barrier(
            slot.command_buffer,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &to_transfer,
        );
        let region = [vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_offset(vk::Offset3D::default())
            .image_extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })];
        device.cmd_copy_image_to_buffer(
            slot.command_buffer,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            slot.buffer,
            &region,
        );
        let to_present = [vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            )];
        device.cmd_pipeline_barrier(
            slot.command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &to_present,
        );
        device.end_command_buffer(slot.command_buffer)?;
    }
    Ok(())
}

fn readback_frame(
    device: &ash::Device,
    format: vk::Format,
    extent: vk::Extent2D,
    slot: &CaptureSlot,
) -> Option<CapturedFrame> {
    let pointer = unsafe {
        device
            .map_memory(
                slot.memory,
                0,
                slot.allocation_size,
                vk::MemoryMapFlags::empty(),
            )
            .ok()?
    };
    if !slot.coherent {
        let range = [vk::MappedMemoryRange::default()
            .memory(slot.memory)
            .offset(0)
            .size(vk::WHOLE_SIZE)];
        if unsafe { device.invalidate_mapped_memory_ranges(&range) }.is_err() {
            unsafe { device.unmap_memory(slot.memory) };
            return None;
        }
    }
    let pixels = extent.width as usize * extent.height as usize;
    let input = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), pixels * 4) };
    let mut bgr = Vec::with_capacity(pixels * 3);
    let bgra = matches!(
        format,
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
    );
    for pixel in input.chunks_exact(4) {
        if bgra {
            bgr.extend_from_slice(&pixel[..3]);
        } else {
            bgr.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
    }
    unsafe { device.unmap_memory(slot.memory) };
    Some(CapturedFrame {
        width: extent.width,
        height: extent.height,
        bgr,
    })
}

fn destroy_slot(device: &ash::Device, slot: CaptureSlot) {
    unsafe {
        device.destroy_semaphore(slot.present_semaphore, None);
        device.destroy_fence(slot.fence, None);
        device.destroy_buffer(slot.buffer, None);
        device.free_memory(slot.memory, None);
        device.destroy_command_pool(slot.command_pool, None);
    }
}
