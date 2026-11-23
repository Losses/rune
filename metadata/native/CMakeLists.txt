cmake_minimum_required(VERSION 3.18)
project(rune_taglib LANGUAGES C CXX)

include(FetchContent)

# Use the pinned utfcpp headers without building or installing its CMake project.
FetchContent_Declare(utfcpp
  GIT_REPOSITORY https://github.com/Losses/utfcpp.git
  GIT_TAG 161c216be03e671dbc0ee8cace084d3d82b18932 # 5.0.0
  SOURCE_SUBDIR source)
FetchContent_MakeAvailable(utfcpp)
set(utf8cpp_INCLUDE_DIR "${utfcpp_SOURCE_DIR}/source")
set(CMAKE_DISABLE_FIND_PACKAGE_utf8cpp ON)

# Omit TagLib's upstream utfcpp submodule in favor of the pinned Rune dependency.
FetchContent_Declare(taglib
  GIT_REPOSITORY https://github.com/Losses/taglib.git
  GIT_TAG 7333be146c0a412d345f7e73554e383ebb8d6fc3 # 2.3.2
  GIT_SUBMODULES "")

# TagLib is read-only in Rune and is embedded into Rust's shared or static output.
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(BUILD_BINDINGS OFF CACHE BOOL "" FORCE)
set(WITH_ZLIB OFF CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(taglib)
set(taglib_src "${taglib_SOURCE_DIR}")

add_library(rune_taglib STATIC bridge.cpp)
target_compile_features(rune_taglib PRIVATE cxx_std_17)
target_link_libraries(rune_taglib PRIVATE tag)
target_include_directories(rune_taglib PRIVATE
  "${taglib_src}/taglib" "${taglib_src}/taglib/toolkit" "${taglib_src}/taglib/flac"
  "${taglib_src}/taglib/mp4" "${taglib_src}/taglib/riff"
  "${taglib_src}/taglib/riff/wav" "${taglib_src}/taglib/riff/aiff"
  "${taglib_src}/taglib/mpeg" "${taglib_src}/taglib/ogg"
  "${taglib_src}/taglib/mpeg/id3v2" "${taglib_src}/taglib/mpeg/id3v2/frames"
  "${taglib_src}/taglib/ogg/opus" "${taglib_src}/taglib/ogg/vorbis")
install(TARGETS rune_taglib ARCHIVE DESTINATION "${CMAKE_INSTALL_LIBDIR}")
