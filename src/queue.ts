interface Task {
    id: string
    run: () => Promise<void>
}

export const taskQueue: Task[] = []

let isProcessing = false

// Function to process the queue
async function processQueue() {
    if (isProcessing) return
    isProcessing = true

    while (taskQueue.length > 0) {
        const task = taskQueue.shift()! // Get the first task
        console.log(`Processing task ID: ${task.id}`)
        await task.run() // Execute the task
    }

    isProcessing = false
}

// Add a task to the queue
export function addToQueue(id: string, taskFunc: () => Promise<void>) {
    taskQueue.push({ id, run: taskFunc })
    console.log(`Task added to queue. ID: ${id}`)
    processQueue().then()
}
