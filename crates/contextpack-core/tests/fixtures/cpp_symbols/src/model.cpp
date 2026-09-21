#include "../include/model.hpp"

namespace Acme::Model {

template <typename T>
Box<T>::Box() = default;

template <typename T>
Box<T>::~Box() = default;

template <typename T>
void Box<T>::put(T value)
{
    (void)value;
}

template <typename T>
bool Box<T>::operator==(const Box& other) const
{
    return this == &other;
}

struct Record {
    int value;
};

int overloaded(int value)
{
    return value;
}

int overloaded(double value)
{
    return static_cast<int>(value);
}

} // namespace Acme::Model

namespace {
inline int hidden_helper() noexcept
{
    return 7;
}
}

