#include <atomic>
#include <chrono>
#include <cstdint>
#include <iomanip>
#include <iostream>
#include <numeric>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#include <unistd.h>

namespace {

struct Sample {
    std::uint64_t index;
    std::uint64_t input;
    std::uint64_t mixed;
    std::uint64_t checksum;
};

class Analyzer {
  public:
    explicit Analyzer(std::size_t reserve) {
        samples_.reserve(reserve);
    }

    Sample record(std::uint64_t index, std::uint64_t seed) {
        const auto input = scramble(seed + index * 3);
        const auto mixed = slow_mix(input, 5);
        const auto checksum = rolling_checksum(index, input, mixed);
        Sample sample{index, input, mixed, checksum};
        samples_.push_back(sample);
        return sample;
    }

    std::uint64_t summarize() const {
        return std::accumulate(
            samples_.begin(),
            samples_.end(),
            std::uint64_t{0},
            [](std::uint64_t acc, const Sample& sample) {
                return acc ^ sample.checksum ^ sample.mixed;
            }
        );
    }

    std::size_t count() const {
        return samples_.size();
    }

  private:
    static std::uint64_t scramble(std::uint64_t value) {
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        return value;
    }

    static std::uint64_t slow_mix(std::uint64_t value, int depth) {
        if (depth <= 0) {
            return value ^ 0x9E3779B185EBCA87ULL;
        }
        const auto rotated = (value << 9) | (value >> (64 - 9));
        return slow_mix(rotated ^ (value + static_cast<std::uint64_t>(depth)), depth - 1);
    }

    static std::uint64_t rolling_checksum(
        std::uint64_t index,
        std::uint64_t input,
        std::uint64_t mixed
    ) {
        return (index * 1315423911ULL) ^ (input + 0xA5A5A5A5ULL) ^ (mixed >> 3);
    }

    std::vector<Sample> samples_;
};

struct Config {
    int iterations = 320;
    int sleep_ms = 25;
    std::uint64_t seed = 0xC0FFEEULL;
    bool nonzero_exit = false;
};

Config parse_args(int argc, char** argv) {
    Config config;
    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--iterations" && i + 1 < argc) {
            config.iterations = std::stoi(argv[++i]);
        } else if (arg == "--sleep-ms" && i + 1 < argc) {
            config.sleep_ms = std::stoi(argv[++i]);
        } else if (arg == "--seed" && i + 1 < argc) {
            config.seed = static_cast<std::uint64_t>(std::stoull(argv[++i]));
        } else if (arg == "--nonzero-exit") {
            config.nonzero_exit = true;
        }
    }
    if (config.iterations < 1) {
        config.iterations = 1;
    }
    if (config.sleep_ms < 0) {
        config.sleep_ms = 0;
    }
    return config;
}

std::string format_sample(const Sample& sample) {
    std::ostringstream out;
    out << "sample index=" << sample.index << " input=0x" << std::hex << sample.input
        << " mixed=0x" << sample.mixed << " checksum=0x" << sample.checksum;
    return out.str();
}

}  // namespace

int main(int argc, char** argv) {
    const Config config = parse_args(argc, argv);
    Analyzer analyzer(static_cast<std::size_t>(config.iterations));
    std::vector<std::uint64_t> scratch(64, config.seed);
    std::atomic<bool> keep_worker_alive{true};
    std::atomic<std::uint64_t> worker_counter{0};

    std::thread worker([&] {
        std::size_t cursor = 0;
        while (keep_worker_alive.load()) {
            scratch[cursor] ^= 0x9E3779B97F4A7C15ULL + worker_counter.fetch_add(1);
            cursor = (cursor + 1) % scratch.size();
            std::this_thread::sleep_for(std::chrono::milliseconds(5));
        }
    });

    std::cout << "ready pid=" << getpid() << " iterations=" << config.iterations
              << " sleep_ms=" << config.sleep_ms << std::endl;

    Sample last_sample{};
    for (int i = 0; i < config.iterations; ++i) {
        scratch[static_cast<std::size_t>(i) % scratch.size()] += static_cast<std::uint64_t>(i);
        const auto seed = scratch[static_cast<std::size_t>(i * 7) % scratch.size()] ^ config.seed;
        last_sample = analyzer.record(static_cast<std::uint64_t>(i), seed);

        if ((i % 80) == 0) {
            std::cout << format_sample(last_sample) << std::endl;
        }

        std::this_thread::sleep_for(std::chrono::milliseconds(config.sleep_ms));
    }

    keep_worker_alive = false;
    worker.join();

    const auto summary = analyzer.summarize() ^ worker_counter.load();
    std::cout << "done samples=" << analyzer.count() << " worker_counter=" << worker_counter.load()
              << " summary=0x" << std::hex << summary << std::endl;
    if (config.nonzero_exit) {
        return static_cast<int>(summary & 0xffU);
    }
    return 0;
}
