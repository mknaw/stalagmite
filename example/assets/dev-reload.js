document.addEventListener('DOMContentLoaded', function() {
    // Your code goes here
    console.log("Document is ready!");

    if (typeof (EventSource) !== "undefined") {
        var source = new EventSource("/__dev_reload");

        source.onmessage = () => {
            window.location.reload();
        }
    } else {
        // SSE not supported by browser
        console.log("Server-Sent Events not supported by your browser.");
    }
});
