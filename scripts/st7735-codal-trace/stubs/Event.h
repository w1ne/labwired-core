// SPDX-License-Identifier: MIT
#pragma once
#include <functional>
#include <vector>
namespace codal {
class Event;
void emit_trace_event(const Event& event);
class Event {
public:
    int source;
    int value;
    Event(int source_, int value_) : source(source_), value(value_) {
        emit_trace_event(*this);
    }
};
class EventBus {
public:
    struct Handler { int source; int value; std::function<void(Event)> call; };
    std::vector<Handler> handlers;
    template <typename T>
    void listen(int source, int value, T* object, void (T::*method)(Event)) {
        handlers.push_back({source, value, [object, method](Event e) { (object->*method)(e); }});
    }
};
class EventModel {
public:
    static EventBus* defaultEventBus;
};
}
