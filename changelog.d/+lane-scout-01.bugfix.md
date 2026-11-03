`claim-next` takes the same node for the same graph and the same worker in every process.
It picked from the graph's hash order, which changes from one process to the next, so one worker on one graph got a different node on each call.
