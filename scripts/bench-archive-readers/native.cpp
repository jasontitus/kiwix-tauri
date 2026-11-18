#include <algorithm>
#include <chrono>
#include <cstring>
#include <iostream>
#include <random>
#include <set>
#include <stdexcept>
#include <string>
#include <vector>
#include <zim/archive.h>
#include <zim/entry.h>
#include <zim/item.h>

using Clock = std::chrono::steady_clock;

void number(uint64_t value) {
  char bytes[8];
  for (unsigned i = 0; i != 8; ++i)
    bytes[i] = char(value >> (i * 8));
  std::cout.write(bytes, sizeof(bytes));
}
void field(const char *data, size_t size) {
  if (size > 65536)
    throw std::runtime_error("result field too large");
  number(size);
  if (size)
    std::cout.write(data, size);
}
bool clean(const std::string &value) {
  return !value.empty() && value.size() <= 4096 &&
         value.find_first_of("\t\r\n") == std::string::npos;
}
void choose(const zim::Archive &archive) {
  std::mt19937_64 random(20261118);
  std::set<uint32_t> visited;
  unsigned html = 0, media = 0;
  const auto total = archive.getEntryCount();
  for (unsigned j = 0; total && j < 10000 && (html < 24 || media < 8); ++j) {
    auto row = static_cast<uint32_t>(random() % total);
    if (!visited.insert(row).second)
      continue;
    auto entry = archive.getEntryByPath(row);
    if (entry.isRedirect() || !clean(entry.getPath()) ||
        !clean(entry.getTitle()))
      continue;
    auto item = entry.getItem();
    const auto mime = item.getMimetype();
    const bool article = mime == "text/html" || mime == "application/xhtml+xml";
    const bool av = mime.find("image/") == 0 || mime.find("audio/") == 0 ||
                    mime.find("video/") == 0;
    if (!(article && html < 24) && !(av && media < 8))
      continue;
    const auto size = item.getSize();
    if (article && size < 256)
      continue;
    if (av && size < 8192)
      continue;
    std::cout << (article ? "html" : "media") << '\t' << size << '\t'
              << entry.getPath() << '\t' << entry.getTitle() << '\n';
    if (article)
      ++html;
    else
      ++media;
  }
}

int main(int argc, char **argv) try {
  if (argc != 5)
    throw std::runtime_error(
        "expected mode, archive, cache size and preload flag");
  if (std::string(argv[3]) != "default")
    zim::setClusterCacheMaxSize(std::stoull(argv[3]));
  const bool preload = std::string(argv[4]) == "eager";
  std::ios::sync_with_stdio(false);
  std::cin.tie(nullptr);
  const std::string mode = argv[1];
  const auto start = Clock::now();
  zim::Archive archive(argv[2], zim::OpenConfig().preloadXapianDb(preload));
  const auto opened = Clock::now() - start;
  auto elapsed = opened;
  if (mode == "choose") {
    choose(archive);
    return 0;
  }
  if (mode == "open") {
    number(archive.getAllEntryCount());
  } else {
    elapsed = Clock::duration::zero();
    std::vector<char> scratch(65536);
    std::string line;
    bool warm = mode == "repeat";
    while (std::getline(std::cin, line)) {
      const auto first = line.find('\t');
      const auto second =
          line.find('\t', first == std::string::npos ? first : first + 1);
      if (second == std::string::npos)
        throw std::runtime_error("invalid request");
      const auto offset = std::stoull(line.substr(0, first));
      const auto size = std::stoull(line.substr(first + 1, second - first - 1));
      const auto key = line.substr(second + 1);
      const auto before = Clock::now();
      if (mode == "read" || mode == "repeat") {
        if (size > scratch.size())
          throw std::runtime_error("oversized read");
        const auto item = archive.getEntryByPath(key).getItem(true);
        const auto blob = item.getData(offset, size);
        const auto n = blob.size();
        if (n > size)
          throw std::runtime_error("unexpected blob size");
        if (n)
          std::memcpy(scratch.data(), blob.data(), n);
        if (warm) {
          warm = false;
          continue;
        }
        elapsed += Clock::now() - before;
        field(scratch.data(), n);
      } else if (mode == "title") {
        auto matches = archive.findByTitle(key);
        auto count = matches.size();
        std::vector<std::pair<std::string, std::string>> rows;
        for (const auto &entry : matches.offset(0, 4))
          rows.emplace_back(entry.getPath(), entry.getTitle());
        elapsed += Clock::now() - before;
        number(count);
        for (const auto &row : rows) {
          field(row.first.data(), row.first.size());
          field(row.second.data(), row.second.size());
        }
      } else
        throw std::runtime_error("unknown mode");
    }
  }
  std::cout.flush();
  if (!std::cout)
    throw std::runtime_error("output failed");
  std::cerr
      << "OPEN_NS "
      << std::chrono::duration_cast<std::chrono::nanoseconds>(opened).count()
      << '\n';
  std::cerr << "CACHE " << zim::getClusterCacheMaxSize() << ' '
            << zim::getClusterCacheCurrentSize() << '\n';
  std::cerr
      << "BENCH_NS "
      << std::chrono::duration_cast<std::chrono::nanoseconds>(elapsed).count()
      << '\n';
  return 0;
} catch (const std::exception &error) {
  std::cerr << error.what() << '\n';
  return 2;
}
