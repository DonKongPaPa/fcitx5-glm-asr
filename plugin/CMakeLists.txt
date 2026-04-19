add_library(glm-asr SHARED
    glm_asr_addon.cpp
)

find_package(Fcitx5Utils REQUIRED)

target_link_libraries(glm-asr PRIVATE Fcitx5::Core Fcitx5::Config Fcitx5::Utils)
target_include_directories(glm-asr PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
set_target_properties(glm-asr PROPERTIES PREFIX "")

configure_file(glm-asr-addon.conf.in ${CMAKE_CURRENT_BINARY_DIR}/glm-asr.conf)

install(TARGETS glm-asr DESTINATION "${FCITX_INSTALL_LIBDIR}/fcitx5")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/glm-asr.conf"
        DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon")
