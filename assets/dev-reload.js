document.addEventListener("DOMContentLoaded", function() {
    if (typeof (EventSource) !== "undefined") {
        var source = new EventSource("/__dev_reload");
        console.log("Serving with dev mode - listening to reload SSEs...")

        source.onmessage = () => {
            window.location.reload();
        }
    } else {
        // SSE not supported by browser
        console.log("Server-Sent Events not supported by your browser.");
    }
});
