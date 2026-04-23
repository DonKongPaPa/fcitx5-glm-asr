cmake_minimum_required(VERSION 3.21)
project(fcitx5-glm-asr VERSION 0.1.2)

find_package(Fcitx5Core REQUIRED)
include("${FCITX_INSTALL_CMAKECONFIG_DIR}/Fcitx5Utils/Fcitx5CompilerSettings.cmake")

add_subdirectory(plugin)
