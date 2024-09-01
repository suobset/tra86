#include "tra86_backend.h"

void startCBuild(std::string fileName) {
    // By this point, you should have a string with the whole Path
    // Yes, the name "fileName" can be a bit misleading.
    // Example input:
    // /Users/kushagrasrivastava/Documents/morbios/README.md

    // Find the position of the last '/' in the file path
    size_t lastSlashPos = fileName.find_last_of("/");

    // Divide the string into filePath and actualFile
    std::string filePath = fileName.substr(0, lastSlashPos);
    std::string actualFile = fileName.substr(lastSlashPos + 1);

    // Construct the command string for 'ls -la'
    std::string command = "ls -la \"" + filePath + "\"";

    // Execute the command using system()
    int result = system(command.c_str());

    // Check if the command was successful
    if (result == 0) {
        std::cout << "Command executed successfully." << std::endl;
    } else {
        std::cerr << "Command execution failed with error code: " << result << std::endl;
    }

}
