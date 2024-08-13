# Additional clean files
cmake_minimum_required(VERSION 3.16)

if("${CONFIG}" STREQUAL "" OR "${CONFIG}" STREQUAL "Debug")
  file(REMOVE_RECURSE
  "CMakeFiles/tra86_autogen.dir/AutogenUsed.txt"
  "CMakeFiles/tra86_autogen.dir/ParseCache.txt"
  "tra86_autogen"
  )
endif()
