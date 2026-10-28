cmake_minimum_required(VERSION 3.16)
project(CerberusMCP LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
endif()

# Keep every binary in one folder so the loader finds assets/ next to it.
# (per-config variables stop multi-config generators from adding a /<Config> folder)
function(cm_set_outdir target dir)
    set_target_properties(${target} PROPERTIES
        RUNTIME_OUTPUT_DIRECTORY ${dir}
        RUNTIME_OUTPUT_DIRECTORY_DEBUG ${dir}
        RUNTIME_OUTPUT_DIRECTORY_RELEASE ${dir}
        RUNTIME_OUTPUT_DIRECTORY_RELWITHDEBINFO ${dir}
        RUNTIME_OUTPUT_DIRECTORY_MINSIZEREL ${dir})
endfunction()

set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR})
foreach(cfg DEBUG RELEASE RELWITHDEBINFO MINSIZEREL)
    set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_${cfg} ${CMAKE_BINARY_DIR})
endforeach()

set(IMGUI ${CMAKE_CURRENT_SOURCE_DIR}/external/imgui)
set(IMGUI_CORE
    ${IMGUI}/imgui.cpp
    ${IMGUI}/imgui_draw.cpp
    ${IMGUI}/imgui_tables.cpp
    ${IMGUI}/imgui_widgets.cpp)

set(COMMON_INCLUDES ${IMGUI} ${IMGUI}/backends)
set(COMMON_DEFS IMGUI_DEFINE_MATH_OPERATORS)

if(WIN32)
    # Windows keeps the original Win32 + DirectX 11 path (also what build.bat uses).
    set(LOADER_PLATFORM src/host_win32.cpp)
    set(PLATFORM_BACKENDS ${IMGUI}/backends/imgui_impl_win32.cpp ${IMGUI}/backends/imgui_impl_dx11.cpp)
    set(PLATFORM_LIBS d3d11 dxgi d3dcompiler dwmapi windowscodecs ole32 shlwapi)
    # rust static core (tokio/windows-sys/system deps)
    set(CERBERUS_SYS_LIBS ws2_32 userenv bcrypt ntdll advapi32 iphlpapi psapi shell32)
else()
    # macOS & Linux: GLFW window + OpenGL 3 renderer.
    set(LOADER_PLATFORM src/host_glfw.cpp)
    set(PLATFORM_BACKENDS ${IMGUI}/backends/imgui_impl_glfw.cpp ${IMGUI}/backends/imgui_impl_opengl3.cpp)

    find_package(OpenGL REQUIRED)
    find_package(glfw3 3.3 QUIET)
    if(TARGET glfw)
        set(GLFW_LIB glfw)
    elseif(TARGET glfw3)
        set(GLFW_LIB glfw3)
    else()
        find_package(PkgConfig REQUIRED)
        pkg_check_modules(GLFW REQUIRED IMPORTED_TARGET glfw3)
        set(GLFW_LIB PkgConfig::GLFW)
    endif()
    set(PLATFORM_LIBS ${GLFW_LIB} OpenGL::GL)
    set(CERBERUS_SYS_LIBS "")
    if(APPLE)
        list(APPEND PLATFORM_LIBS "-framework Cocoa" objc)
        list(APPEND CERBERUS_SYS_LIBS "-framework Security" "-framework CoreFoundation" "-framework SystemConfiguration")
    endif()
endif()

# ---------------------------------------------------------------- loader
add_executable(CerberusMCP
    ${LOADER_PLATFORM}
    src/app.cpp
    src/config.cpp
    src/cerberus_api.cpp
    src/registry.cpp
    src/rpc.cpp
    src/lang/lang.cpp
    src/ui/core.cpp
    src/ui/flags.cpp
    src/ui/icons.cpp
    src/ui/widgets.cpp
    ${IMGUI_CORE}
    ${IMGUI}/misc/cpp/imgui_stdlib.cpp
    ${PLATFORM_BACKENDS})
target_include_directories(CerberusMCP PRIVATE ${COMMON_INCLUDES} src external/stb external)
target_compile_definitions(CerberusMCP PRIVATE ${COMMON_DEFS})

# ---------------------------------------------------------------- rust core
set(CERBERUS_ROOT ${CMAKE_CURRENT_SOURCE_DIR}/..)
set(CERBERUS_TARGET_DIR ${CERBERUS_ROOT}/target/release)
if(WIN32)
    set(CERBERUS_LIB ${CERBERUS_TARGET_DIR}/cerberus_mcp.lib)
else()
    set(CERBERUS_LIB ${CERBERUS_TARGET_DIR}/libcerberus_mcp.a)
endif()
file(GLOB_RECURSE CERBERUS_RS CONFIGURE_DEPENDS
    ${CERBERUS_ROOT}/src/*.rs
    ${CERBERUS_ROOT}/Cargo.toml
    ${CERBERUS_ROOT}/Cargo.lock)
add_custom_command(OUTPUT ${CERBERUS_LIB}
    COMMAND cargo build --release --manifest-path ${CERBERUS_ROOT}/Cargo.toml
    DEPENDS ${CERBERUS_RS}
    WORKING_DIRECTORY ${CERBERUS_ROOT}
    COMMENT "cargo build --release (cerberus-mcp static core)"
    VERBATIM)
add_custom_target(cerberus_core DEPENDS ${CERBERUS_LIB})
add_dependencies(CerberusMCP cerberus_core)

target_link_libraries(CerberusMCP PRIVATE
    ${CERBERUS_LIB}
    ${CERBERUS_SYS_LIBS}
    ${PLATFORM_LIBS})

# assets/ next to the loader binary
add_custom_command(TARGET CerberusMCP POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_directory
            ${CMAKE_CURRENT_SOURCE_DIR}/assets ${CMAKE_BINARY_DIR}/assets
    COMMENT "Copying assets")
