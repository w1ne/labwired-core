find_package(PkgConfig QUIET)
if(PkgConfig_FOUND)
    pkg_check_modules(CMOCKA QUIET cmocka)
endif()

if(NOT CMOCKA_FOUND)
    find_path(CMOCKA_INCLUDE_DIRS cmocka.h)
    find_library(CMOCKA_LIBRARIES cmocka)
    if(CMOCKA_INCLUDE_DIRS AND CMOCKA_LIBRARIES)
        set(CMOCKA_FOUND TRUE)
    endif()
endif()

if(CMOCKA_FOUND)
    message(STATUS "CMocka found: ${CMOCKA_LIBRARIES}")
    include_directories(${CMOCKA_INCLUDE_DIRS})

    add_executable(test_master_startup test_master_startup.c)
    target_link_libraries(test_master_startup iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_startup COMMAND test_master_startup)

    add_executable(test_master_pd test_master_pd.c)
    target_link_libraries(test_master_pd iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_pd COMMAND test_master_pd)

    add_executable(test_master_isdu test_master_isdu.c)
    target_link_libraries(test_master_isdu iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_isdu COMMAND test_master_isdu)

    add_executable(test_master_isdu_public test_master_isdu_public.c)
    target_link_libraries(test_master_isdu_public iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_isdu_public COMMAND test_master_isdu_public)

    add_executable(test_master_tick test_master_tick.c)
    target_link_libraries(test_master_tick iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_tick COMMAND test_master_tick)

    add_executable(test_master_controller test_master_controller.c)
    target_link_libraries(test_master_controller iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_controller COMMAND test_master_controller)

    add_executable(test_master_parameters test_master_parameters.c)
    target_link_libraries(test_master_parameters iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_parameters COMMAND test_master_parameters)

    add_executable(test_master_public_flow test_master_public_flow.c)
    target_link_libraries(test_master_public_flow iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_public_flow COMMAND test_master_public_flow)

    add_executable(test_master_sio_public test_master_sio_public.c)
    target_link_libraries(test_master_sio_public iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_sio_public COMMAND test_master_sio_public)

    add_executable(test_master_fake_device test_master_fake_device.c fake_iolink_device.c)
    target_link_libraries(test_master_fake_device iolinki_master ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_fake_device COMMAND test_master_fake_device)

    add_library(iolinki_device_stack_for_tests STATIC
        "${IOLINKI_DEVICE_DIR}/src/device.c"
        "${IOLINKI_DEVICE_DIR}/src/phy_generic.c"
        "${IOLINKI_DEVICE_DIR}/src/phy_virtual.c"
        "${IOLINKI_DEVICE_DIR}/src/dll.c"
        "${IOLINKI_DEVICE_DIR}/src/isdu.c"
        "${IOLINKI_DEVICE_DIR}/src/events.c"
        "${IOLINKI_DEVICE_DIR}/src/platform.c"
        "${IOLINKI_DEVICE_DIR}/src/params.c"
        "${IOLINKI_DEVICE_DIR}/src/data_storage.c"
        "${IOLINKI_DEVICE_DIR}/src/device_info.c"
        "${IOLINKI_DEVICE_DIR}/src/platform_stubs.c"
        "${IOLINKI_DEVICE_DIR}/src/platform/linux/time_utils.c"
        "${IOLINKI_DEVICE_DIR}/src/platform/linux/nvm_mock.c"
    )
    target_include_directories(iolinki_device_stack_for_tests PUBLIC "${IOLINKI_DEVICE_DIR}/include")
    target_link_libraries(iolinki_device_stack_for_tests PUBLIC iolinki_frame_helpers)

    add_executable(test_master_real_iolinki_device test_master_real_iolinki_device.c)
    target_link_libraries(test_master_real_iolinki_device iolinki_master iolinki_device_stack_for_tests ${CMOCKA_LIBRARIES})
    add_test(NAME test_master_real_iolinki_device COMMAND test_master_real_iolinki_device)

else()
    message(WARNING "CMocka not found, unit tests will be skipped. Install libcmocka-dev to enable them.")
endif()

add_executable(test_master_public_header test_master_public_header.c)
target_link_libraries(test_master_public_header iolinki_master)
add_test(NAME test_master_public_header COMMAND test_master_public_header)
