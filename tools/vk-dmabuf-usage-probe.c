// Which usage bits does the driver allow on a LINEAR BGRA8 dma-buf image?
// Asks vkGetPhysicalDeviceImageFormatProperties2 for every subset of the
// four bits target.rs:EXPORT_IMAGE_USAGE uses, and reports EXPORTABLE /
// IMPORTABLE, for TILING_LINEAR and (if the device has the modifier
// extension) TILING_DRM_FORMAT_MODIFIER with DRM_FORMAT_MOD_LINEAR.
// Build: cc -O2 -o vk-dmabuf-usage-probe vk-dmabuf-usage-probe.c -lvulkan
#include <stdio.h>
#include <string.h>
#include <vulkan/vulkan.h>

static const struct { VkImageUsageFlags bit; const char *name; } BITS[] = {
    { VK_IMAGE_USAGE_TRANSFER_SRC_BIT,     "TRANSFER_SRC" },
    { VK_IMAGE_USAGE_TRANSFER_DST_BIT,     "TRANSFER_DST" },
    { VK_IMAGE_USAGE_SAMPLED_BIT,          "SAMPLED" },
    { VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT, "COLOR_ATTACHMENT" },
};

static void usage_str(VkImageUsageFlags u, char *out, size_t n) {
    out[0] = '\0';
    for (int i = 0; i < 4; i++)
        if (u & BITS[i].bit) {
            if (out[0]) strncat(out, "|", n - strlen(out) - 1);
            strncat(out, BITS[i].name, n - strlen(out) - 1);
        }
    if (!out[0]) strncat(out, "(none)", n - 1);
}

int main(void) {
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .apiVersion = VK_API_VERSION_1_1 };
    VkInstanceCreateInfo ici = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                 .pApplicationInfo = &app };
    VkInstance inst;
    if (vkCreateInstance(&ici, NULL, &inst) != VK_SUCCESS) {
        fprintf(stderr, "vkCreateInstance failed\n");
        return 1;
    }
    uint32_t ndev = 0;
    vkEnumeratePhysicalDevices(inst, &ndev, NULL);
    VkPhysicalDevice devs[8];
    if (ndev > 8) ndev = 8;
    vkEnumeratePhysicalDevices(inst, &ndev, devs);

    for (uint32_t d = 0; d < ndev; d++) {
        VkPhysicalDeviceProperties p;
        vkGetPhysicalDeviceProperties(devs[d], &p);
        if (p.deviceType == VK_PHYSICAL_DEVICE_TYPE_CPU) continue;

        uint32_t next = 0;
        vkEnumerateDeviceExtensionProperties(devs[d], NULL, &next, NULL);
        VkExtensionProperties exts[512];
        if (next > 512) next = 512;
        vkEnumerateDeviceExtensionProperties(devs[d], NULL, &next, exts);
        int has_mod = 0;
        for (uint32_t i = 0; i < next; i++)
            if (!strcmp(exts[i].extensionName, VK_EXT_IMAGE_DRM_FORMAT_MODIFIER_EXTENSION_NAME))
                has_mod = 1;

        printf("device: %s (vendor 0x%04x device 0x%04x) image_drm_format_modifier=%s\n",
               p.deviceName, p.vendorID, p.deviceID, has_mod ? "yes" : "NO");
        for (int pass = 0; pass < 2; pass++) {
            if (pass == 1 && !has_mod) break;
            printf("  %s + B8G8R8A8_UNORM + DMA_BUF:\n",
                   pass ? "DRM_FORMAT_MODIFIER(LINEAR)" : "TILING_LINEAR");
            for (VkImageUsageFlags u = 1; u < 16; u++) {
                VkImageUsageFlags usage = 0;
                for (int i = 0; i < 4; i++)
                    if (u & (1u << i)) usage |= BITS[i].bit;

                VkPhysicalDeviceImageDrmFormatModifierInfoEXT mod = {
                    .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT,
                    .drmFormatModifier = 0 /* DRM_FORMAT_MOD_LINEAR */,
                    .sharingMode = VK_SHARING_MODE_EXCLUSIVE };
                VkPhysicalDeviceExternalImageFormatInfo ext = {
                    .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO,
                    .pNext = pass ? &mod : NULL,
                    .handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT };
                VkPhysicalDeviceImageFormatInfo2 info = {
                    .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2,
                    .pNext = &ext,
                    .format = VK_FORMAT_B8G8R8A8_UNORM,
                    .type = VK_IMAGE_TYPE_2D,
                    .tiling = pass ? VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT : VK_IMAGE_TILING_LINEAR,
                    .usage = usage };
                VkExternalImageFormatProperties eprops = {
                    .sType = VK_STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES };
                VkImageFormatProperties2 props = {
                    .sType = VK_STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2, .pNext = &eprops };

                VkResult r = vkGetPhysicalDeviceImageFormatProperties2(devs[d], &info, &props);
                char us[96];
                usage_str(usage, us, sizeof us);
                VkExternalMemoryFeatureFlags f = eprops.externalMemoryProperties.externalMemoryFeatures;
                if (r == VK_SUCCESS)
                    printf("    %-52s OK   export=%s import=%s dedicated_only=%s\n", us,
                           (f & VK_EXTERNAL_MEMORY_FEATURE_EXPORTABLE_BIT) ? "yes" : "no",
                           (f & VK_EXTERNAL_MEMORY_FEATURE_IMPORTABLE_BIT) ? "yes" : "no",
                           (f & VK_EXTERNAL_MEMORY_FEATURE_DEDICATED_ONLY_BIT) ? "yes" : "no");
                else
                    printf("    %-52s FAIL (%d)\n", us, r);
            }
        }
    }
    vkDestroyInstance(inst, NULL);
    return 0;
}
