#pragma once

namespace Acme::Model {

template <typename T>
class Box {
public:
    Box();
    ~Box();
    void put(T value);
    bool operator==(const Box& other) const;
};

struct Record;

enum class State {
    Ready,
    Failed,
};

int overloaded(int value);
int overloaded(double value);

} // namespace Acme::Model

